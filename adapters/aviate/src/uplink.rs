//! DJI-style flight uplink: stick positions become velocity setpoints
//! (issue #12).
//!
//! The control law, not the stick map, is what makes a camera drone feel
//! like one: sticks command **velocities**, centered sticks command
//! zero, and the FC's velocity mode brakes to a hover when input stops.
//! This module turns canonical `[-1, 1]` axes into
//! SET_POSITION_TARGET_LOCAL_NED frames: horizontal sticks are
//! body-frame velocity demands rotated into NED by the vehicle's current
//! yaw, the yaw stick is a rate demand integrated on the ground into an
//! absolute heading setpoint (Aviate's velocity mode takes absolute
//! yaw), and throttle is a climb-rate demand.

use std::net::{SocketAddr, UdpSocket};
use std::time::Instant;

use tracing::{info, warn};

use crate::mavlink::{encode_arm_command, encode_velocity_setpoint};

/// Full-stick horizontal velocity demand.
const MAX_HORIZONTAL_MPS: f32 = 3.0;
/// Full-stick climb/descend rate demand.
const MAX_VERTICAL_MPS: f32 = 1.5;
/// Full-stick yaw rate demand (~86°/s).
const MAX_YAW_RATE_RPS: f32 = 1.5;
/// Longest believable gap between control frames when integrating the
/// yaw-rate stick; anything longer is a stall, not a dt.
const MAX_DT_S: f32 = 0.1;
/// Stick frames are suppressed this long after an arm/disarm send: the
/// FC stages inbound commands in a single slot, so a setpoint arriving
/// in the same poll batch would overwrite the arm before the control
/// loop consumes it.
const ARM_QUIET: std::time::Duration = std::time::Duration::from_millis(150);

/// The UDP MAVLink command uplink to the FC.
#[derive(Debug)]
pub struct FlightUplink {
    socket: UdpSocket,
    target: SocketAddr,
    seq: u8,
    heading_sp_rad: f32,
    last_frame: Option<Instant>,
    quiet_until: Option<Instant>,
    // Motors-idle gate: after arm, no velocity setpoints stream until
    // the first deliberate climb input. Streaming vz=0 to a grounded
    // vehicle commands "hold zero vertical velocity" at near-hover
    // thrust, which tips it over — real drones idle until the first
    // climb, so this does too.
    airborne: bool,
    started: Instant,
    send_failures: u64,
}

/// Climb-stick threshold that opens the motors-idle gate after arming.
const TAKEOFF_STICK: f32 = 0.15;

impl FlightUplink {
    /// Binds an ephemeral socket toward the FC's command port
    /// (`PILOTAGE_AVIATE_FC_ADDR`, default `127.0.0.1:20000` — the SITL
    /// FC's MAVLink/GCS port).
    ///
    /// # Errors
    ///
    /// Returns the socket bind error.
    pub fn new() -> std::io::Result<Self> {
        let target = std::env::var("PILOTAGE_AVIATE_FC_ADDR")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 20000)));
        let socket = UdpSocket::bind("127.0.0.1:0")?;
        info!(%target, "Aviate flight uplink ready");
        Ok(Self {
            socket,
            target,
            seq: 0,
            heading_sp_rad: 0.0,
            last_frame: None,
            quiet_until: None,
            airborne: false,
            started: Instant::now(),
            send_failures: 0,
        })
    }

    fn send(&mut self, frame: &[u8]) {
        if self.socket.send_to(frame, self.target).is_err() {
            self.send_failures = self.send_failures.wrapping_add(1);
            if self.send_failures == 1 || self.send_failures.is_multiple_of(100) {
                warn!(
                    failures = self.send_failures,
                    target = %self.target,
                    "flight uplink send failed"
                );
            }
        }
        self.seq = self.seq.wrapping_add(1);
    }

    /// Sends arm/disarm and re-seeds the heading setpoint from the
    /// vehicle's current yaw so the first yaw-stick input turns from
    /// where the aircraft actually points.
    pub fn send_arm(&mut self, arm: bool, current_yaw_rad: f32) {
        self.heading_sp_rad = current_yaw_rad;
        self.last_frame = None;
        self.quiet_until = Some(Instant::now() + ARM_QUIET);
        self.airborne = false;
        let frame = encode_arm_command(self.seq, arm);
        self.send(&frame);
        info!(arm, "sent arm command to FC");
    }

    /// Converts one canonical stick frame (`[-1, 1]` roll/pitch/
    /// throttle/yaw, stick conventions: pitch + = forward, roll + =
    /// right, throttle + = climb, yaw + = clockwise) into a velocity
    /// setpoint and sends it.
    pub fn send_stick_frame(
        &mut self,
        roll: f32,
        pitch: f32,
        throttle: f32,
        yaw: f32,
        current_yaw_rad: f32,
    ) {
        let now = Instant::now();
        if let Some(quiet) = self.quiet_until {
            if now < quiet {
                return;
            }
            self.quiet_until = None;
        }
        if !self.airborne {
            if throttle <= TAKEOFF_STICK {
                return; // motors idle until the first climb input
            }
            self.airborne = true;
            info!("takeoff: climb input opens the setpoint stream");
        }
        let dt = self
            .last_frame
            .map_or(0.0, |t| now.duration_since(t).as_secs_f32())
            .clamp(0.0, MAX_DT_S);
        self.last_frame = Some(now);

        self.heading_sp_rad = wrap_pi(self.heading_sp_rad + yaw * MAX_YAW_RATE_RPS * dt);

        // Horizontal sticks demand velocity in the vehicle's heading
        // frame; rotate into NED by the *measured* yaw so "stick
        // forward" is always "toward the nose".
        let fwd = pitch * MAX_HORIZONTAL_MPS;
        let lat = roll * MAX_HORIZONTAL_MPS;
        let (sin_y, cos_y) = current_yaw_rad.sin_cos();
        let vel_ned = [
            fwd * cos_y - lat * sin_y,
            fwd * sin_y + lat * cos_y,
            -throttle * MAX_VERTICAL_MPS,
        ];
        let time_boot_ms = self.started.elapsed().as_millis() as u32;
        let frame = encode_velocity_setpoint(self.seq, time_boot_ms, vel_ned, self.heading_sp_rad);
        self.send(&frame);
    }

    /// Sends a zero-velocity setpoint holding the current heading — the
    /// link-loss neutralize action (the FC's velocity mode brakes to a
    /// hover on zero demand).
    pub fn send_neutral(&mut self) {
        let time_boot_ms = self.started.elapsed().as_millis() as u32;
        let frame = encode_velocity_setpoint(self.seq, time_boot_ms, [0.0; 3], self.heading_sp_rad);
        self.send(&frame);
    }

    /// The socket's local address, for tests.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// Overrides the target, for tests.
    #[cfg(test)]
    pub(crate) fn set_target(&mut self, target: SocketAddr) {
        self.target = target;
    }
}

fn wrap_pi(rad: f32) -> f32 {
    let mut r = rad;
    while r > core::f32::consts::PI {
        r -= 2.0 * core::f32::consts::PI;
    }
    while r < -core::f32::consts::PI {
        r += 2.0 * core::f32::consts::PI;
    }
    r
}
