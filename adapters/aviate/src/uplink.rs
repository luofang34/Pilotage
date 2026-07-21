//! DJI-style flight uplink: stick positions become velocity setpoints.
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

use pilotage_mavlink::codec::{
    encode_arm_command, encode_attitude_setpoint, encode_position_setpoint,
    encode_velocity_setpoint,
};

mod fc_replies;

/// Full-stick horizontal velocity demand.
pub(crate) const MAX_HORIZONTAL_MPS: f32 = 3.0;
/// FPV mode: full-stick roll/pitch attitude demand (~35°).
const FPV_MAX_TILT_RAD: f32 = 0.6;
/// FPV mode: thrust at centered throttle stick. Full up commands 1.0,
/// full down 0.30 (enough authority to descend briskly while keeping
/// the props spinning well clear of the mixer floor).
const FPV_HOVER_THRUST: f32 = 0.72;
const FPV_MIN_THRUST: f32 = 0.30;
/// Full-stick climb/descend rate demand.
pub(crate) const MAX_VERTICAL_MPS: f32 = 1.5;
/// Full-stick yaw rate demand (~52°/s). Bounded by what the X500's
/// rotor-drag yaw plant can *stop*: the FC brakes a spin at roughly
/// 0.5 rad/s², so releasing the stick at this rate overshoots the
/// captured heading by well under a half-turn. Higher demands coast
/// through the target and re-chase it the long way around the wrap.
pub(crate) const MAX_YAW_RATE_RPS: f32 = 0.9;
/// Longest believable gap between control frames when integrating the
/// yaw-rate stick; anything longer is a stall, not a dt.
const MAX_DT_S: f32 = 0.1;
/// Measured speed below which the hold point may be captured after the
/// sticks center. Capturing while still moving commands PositionHold on
/// a point the vehicle is about to overrun — it brakes past it, then
/// flies back (~1.5–2 m from a full-stick release). Above this speed
/// the uplink streams zero-velocity setpoints and lets the FC's
/// velocity mode brake; this is a threshold on a measurement, not a
/// control gain — the hold loop itself stays on the FC.
const HOLD_CAPTURE_MAX_MPS: f32 = 0.3;
/// Stick frames are suppressed this long after an arm/disarm send: the
/// FC stages inbound commands in a single slot, so a setpoint arriving
/// in the same poll batch would overwrite the arm before the control
/// loop consumes it.
const ARM_QUIET: std::time::Duration = std::time::Duration::from_millis(150);

/// The uplink's time source. Production reads the system monotonic clock;
/// tests substitute a manually advanced instant so timing behavior (the
/// post-arm quiet window, the slew-limiter dt) is exercised without
/// real-time sleeps.
#[derive(Debug)]
enum UplinkClock {
    System,
    #[cfg(test)]
    Manual(Instant),
}

impl UplinkClock {
    fn now(&self) -> Instant {
        match self {
            Self::System => Instant::now(),
            #[cfg(test)]
            Self::Manual(at) => *at,
        }
    }
}

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
    // Brake-then-hold state: the captured hold point while every stick
    // is centered, and the slew-limited velocity command.
    hold_pos_ned: Option<[f32; 3]>,
    last_vel_ned: [f32; 3],
    started: Instant,
    clock: UplinkClock,
    send_failures: u64,
    expected_system_id: u8,
    expected_component_id: u8,
}

/// Climb-stick threshold that opens the motors-idle gate after arming.
const TAKEOFF_STICK: f32 = 0.15;
/// Acceleration limit shaping stick steps into velocity ramps.
const MAX_ACCEL_MPS2: f32 = 5.0;

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
        socket.set_nonblocking(true)?;
        info!(%target, "Aviate flight uplink ready");
        Ok(Self {
            socket,
            target,
            seq: 0,
            heading_sp_rad: 0.0,
            last_frame: None,
            quiet_until: None,
            airborne: false,
            hold_pos_ned: None,
            last_vel_ned: [0.0; 3],
            started: Instant::now(),
            clock: UplinkClock::System,
            send_failures: 0,
            expected_system_id: 1,
            expected_component_id: 1,
        })
    }

    /// Switches to the manually advanced clock, anchored at construction
    /// time, so tests drive the quiet window and slew dt deterministically.
    #[cfg(test)]
    pub(crate) fn use_manual_clock(&mut self) {
        self.clock = UplinkClock::Manual(self.started);
    }

    /// Advances the manual clock; a no-op on the system clock.
    #[cfg(test)]
    pub(crate) fn advance_clock(&mut self, dt: std::time::Duration) {
        if let UplinkClock::Manual(at) = &mut self.clock {
            *at += dt;
        }
    }

    /// Selects the MAVLink system/component whose replies may affect state.
    pub fn set_expected_source(&mut self, system_id: u8, component_id: u8) {
        self.expected_system_id = system_id;
        self.expected_component_id = component_id;
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

    /// Arms and re-seeds heading from the measured yaw.
    pub fn send_arm(&mut self, current_yaw_rad: f32) {
        self.heading_sp_rad = current_yaw_rad;
        self.send_arm_command(true);
    }

    /// Disarms without requiring a measurement that may have failed.
    pub fn send_disarm(&mut self) {
        self.send_arm_command(false);
    }

    fn send_arm_command(&mut self, arm: bool) {
        self.last_frame = None;
        self.quiet_until = Some(self.clock.now() + ARM_QUIET);
        self.airborne = false;
        self.hold_pos_ned = None;
        self.last_vel_ned = [0.0; 3];
        let frame = encode_arm_command(
            self.seq,
            arm,
            self.expected_system_id,
            self.expected_component_id,
        );
        self.send(&frame);
        info!(arm, "sent arm command to FC");
    }

    /// Converts one canonical stick frame into an FPV attitude
    /// setpoint: roll/pitch sticks command angles, the yaw stick integrates
    /// a heading setpoint exactly like camera mode, and throttle maps directly
    /// to collective thrust around the hover point — altitude is the
    /// pilot's axis in FPV.
    pub fn send_fpv_frame(&mut self, roll: f32, pitch: f32, throttle: f32, yaw: f32) {
        let now = self.clock.now();
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
            info!("takeoff (FPV): climb input opens the setpoint stream");
        }
        let dt = self
            .last_frame
            .map_or(0.0, |t| now.duration_since(t).as_secs_f32())
            .clamp(0.0, MAX_DT_S);
        self.last_frame = Some(now);
        self.hold_pos_ned = None;
        self.heading_sp_rad = wrap_pi(self.heading_sp_rad + yaw * MAX_YAW_RATE_RPS * dt);
        let thrust = if throttle >= 0.0 {
            FPV_HOVER_THRUST + throttle * (1.0 - FPV_HOVER_THRUST)
        } else {
            FPV_HOVER_THRUST + throttle * (FPV_HOVER_THRUST - FPV_MIN_THRUST)
        };
        let frame = encode_attitude_setpoint(
            self.seq,
            self.clock
                .now()
                .saturating_duration_since(self.started)
                .as_millis() as u32,
            pilotage_mavlink::codec::AttitudeTarget {
                roll_rad: roll * FPV_MAX_TILT_RAD,
                pitch_rad: pitch * FPV_MAX_TILT_RAD,
                yaw_rad: self.heading_sp_rad,
                thrust,
                system_id: self.expected_system_id,
                component_id: self.expected_component_id,
            },
        );
        self.send(&frame);
    }

    /// Converts one canonical stick frame (`[-1, 1]` roll/pitch/
    /// throttle/yaw, stick conventions: pitch + = forward, roll + =
    /// right, throttle + = climb, yaw + = clockwise) into a velocity
    /// setpoint and sends it. `current_vel_ned_mps` is the measured
    /// (estimated) velocity as independently validated data — `None`
    /// when the estimate did not declare it valid or it is non-finite —
    /// used only to decide when braking has finished after the sticks
    /// center.
    #[allow(clippy::too_many_arguments)]
    pub fn send_stick_frame(
        &mut self,
        roll: f32,
        pitch: f32,
        throttle: f32,
        yaw: f32,
        current_yaw_rad: f32,
        current_pos_ned_m: [f32; 3],
        current_vel_ned_mps: Option<[f32; 3]>,
    ) {
        let now = self.clock.now();
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
        let time_boot_ms = self
            .clock
            .now()
            .saturating_duration_since(self.started)
            .as_millis() as u32;

        let sticks_active = roll
            .abs()
            .max(pitch.abs())
            .max(throttle.abs())
            .max(yaw.abs())
            > 0.02;
        if !sticks_active {
            self.send_brake_or_hold(current_pos_ned_m, current_vel_ned_mps, time_boot_ms);
            return;
        }
        self.hold_pos_ned = None;

        self.heading_sp_rad = wrap_pi(self.heading_sp_rad + yaw * MAX_YAW_RATE_RPS * dt);

        // Horizontal sticks demand velocity in the vehicle's heading
        // frame; rotate into NED by the *measured* yaw so "stick
        // forward" is always "toward the nose".
        let fwd = pitch * MAX_HORIZONTAL_MPS;
        let lat = roll * MAX_HORIZONTAL_MPS;
        let (sin_y, cos_y) = current_yaw_rad.sin_cos();
        let target = [
            fwd * cos_y - lat * sin_y,
            fwd * sin_y + lat * cos_y,
            -throttle * MAX_VERTICAL_MPS,
        ];
        // Slew-rate limit the horizontal axes so stick steps become
        // acceleration-limited ramps (part of the DJI feel). The
        // vertical demand passes through instantly: ramping vz from
        // near zero holds a grounded vehicle in the unstable
        // near-hover-thrust regime the takeoff gate exists to avoid.
        let dv_max = MAX_ACCEL_MPS2 * dt.max(1.0 / 60.0);
        for (v, t) in self.last_vel_ned.iter_mut().zip(target).take(2) {
            *v += (t - *v).clamp(-dv_max, dv_max);
        }
        self.last_vel_ned[2] = target[2];
        let frame = encode_velocity_setpoint(
            self.seq,
            time_boot_ms,
            self.last_vel_ned,
            self.heading_sp_rad,
            self.expected_system_id,
            self.expected_component_id,
        );
        self.send(&frame);
    }

    /// DJI brake-then-hold, entered while every stick is centered:
    /// first brake — stream zero-velocity setpoints while the vehicle
    /// still moves — then capture the position once braking finishes
    /// and stream it as the hold point. Capturing at release instead
    /// would command PositionHold on a point the momentum overruns,
    /// and the FC would faithfully fly back to it. The hold loop
    /// itself runs on the FC (PositionHold cascade) — ground code
    /// carries no control gains, only the captured point. Once
    /// captured, the hold point is sticky against gusts (re-capturing
    /// on a velocity blip would let the vehicle drift downwind); any
    /// stick deflection returns to velocity mode.
    fn send_brake_or_hold(
        &mut self,
        current_pos_ned_m: [f32; 3],
        current_vel_ned_mps: Option<[f32; 3]>,
        time_boot_ms: u32,
    ) {
        self.last_vel_ned = [0.0; 3];
        // The hold is captured only on POSITIVE evidence of stillness: a
        // validated, finite velocity at or below the capture threshold.
        // Missing or invalid velocity keeps braking — "stopped" is never
        // inferred from absent data (a NaN comparison is silently false,
        // which would otherwise capture the hold at full speed, the exact
        // defect this state machine exists to prevent).
        let demonstrated_still = current_vel_ned_mps.is_some_and(|vel| {
            (vel[0] * vel[0] + vel[1] * vel[1] + vel[2] * vel[2]).sqrt() <= HOLD_CAPTURE_MAX_MPS
        });
        if self.hold_pos_ned.is_none() && !demonstrated_still {
            let frame = encode_velocity_setpoint(
                self.seq,
                time_boot_ms,
                [0.0; 3],
                self.heading_sp_rad,
                self.expected_system_id,
                self.expected_component_id,
            );
            self.send(&frame);
            return;
        }
        let hold = *self.hold_pos_ned.get_or_insert(current_pos_ned_m);
        let frame = encode_position_setpoint(
            self.seq,
            time_boot_ms,
            hold,
            self.heading_sp_rad,
            self.expected_system_id,
            self.expected_component_id,
        );
        self.send(&frame);
    }

    /// Sends a zero-velocity setpoint holding the current heading — the
    /// link-loss neutralize action (the FC's velocity mode brakes to a
    /// hover on zero demand).
    pub fn send_neutral(&mut self) {
        let time_boot_ms = self
            .clock
            .now()
            .saturating_duration_since(self.started)
            .as_millis() as u32;
        let frame = encode_velocity_setpoint(
            self.seq,
            time_boot_ms,
            [0.0; 3],
            self.heading_sp_rad,
            self.expected_system_id,
            self.expected_component_id,
        );
        self.send(&frame);
    }

    /// The socket's local address, for tests.
    /// The MAVLink (system, component) identity this uplink accepts FC
    /// reports from — the provenance identity for those reports.
    pub fn expected_source(&self) -> (u8, u8) {
        (self.expected_system_id, self.expected_component_id)
    }

    /// Wrapping count of datagrams the socket refused to send. A safety
    /// enactment (link-loss neutralize) compares this across its send:
    /// an increment means the FC never received the command, which must
    /// surface as a typed enactment failure — never as silent success.
    pub fn send_failures(&self) -> u64 {
        self.send_failures
    }

    /// Invalidates any captured position-hold context. A link-loss
    /// transition MUST call this: the hold point was captured under the
    /// lost lease, and the vehicle may have drifted arbitrarily far while
    /// neutralized, so a hold surviving the loss would command recovery
    /// back toward an obsolete point the instant control resumes.
    pub fn clear_hold_state(&mut self) {
        self.hold_pos_ned = None;
        self.last_vel_ned = [0.0; 3];
    }

    /// Whether a position-hold point is currently captured, for tests.
    #[cfg(test)]
    pub(crate) fn hold_captured(&self) -> bool {
        self.hold_pos_ned.is_some()
    }

    /// Plants a captured hold point, for tests exercising the stale-hold
    /// invalidation contract without flying a full trajectory.
    #[cfg(test)]
    pub(crate) fn seed_hold_for_test(&mut self, pos_ned_m: [f32; 3]) {
        self.hold_pos_ned = Some(pos_ned_m);
    }

    /// Expires the post-arm quiet window immediately, so tests advance
    /// past it deterministically instead of sleeping wall-clock time.
    #[cfg(test)]
    pub(crate) fn expire_quiet_for_test(&mut self) {
        self.quiet_until = None;
    }

    /// The local socket address this uplink receives FC replies on.
    ///
    /// # Errors
    ///
    /// Returns the socket introspection error.
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
