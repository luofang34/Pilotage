//! PX4 offboard command uplink: GCS heartbeats, telemetry stream
//! negotiation, the offboard arm sequence, and velocity setpoints.
//!
//! The offboard contract drives the shape of this module: PX4 accepts
//! OFFBOARD mode only while a setpoint stream is already flowing, and
//! it runs its own offboard-loss failsafe the moment the stream stops.
//! Arming therefore is a sequence (stream → warmup → mode → arm), and
//! "neutralize" is deliberately "stop the stream": PX4's failsafe
//! (Hold) is the FC-side guarantee, mirroring how the host treats a
//! silent control holder.

use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use tracing::{info, warn};

use pilotage_mavlink::codec::{
    encode_arm_command, encode_command_long, encode_gcs_heartbeat, encode_velocity_setpoint,
};

/// Full-stick horizontal velocity demand. Demand scaling is operator
/// policy destined for per-vehicle profile configuration; these values
/// match the Aviate adapter's so the two SITL vehicles feel identical.
const MAX_HORIZONTAL_MPS: f32 = 3.0;
/// Full-stick climb/descend rate demand.
const MAX_VERTICAL_MPS: f32 = 1.5;
/// Full-stick yaw rate demand (~52°/s).
const MAX_YAW_RATE_RPS: f32 = 0.9;
/// Acceleration limit shaping stick steps into velocity ramps. The
/// PX4 velocity-only offboard path applies no acceleration shaping of
/// its own, so this slew is the only one in the chain.
const MAX_ACCEL_MPS2: f32 = 5.0;
/// Longest believable gap between control frames when integrating the
/// yaw-rate stick; anything longer is a stall, not a dt.
const MAX_DT_S: f32 = 0.1;
/// GCS heartbeat period: presence for PX4's GCS-connection pre-arm
/// check and its datalink-loss failsafe.
const HEARTBEAT_PERIOD: Duration = Duration::from_secs(1);
/// How long the zero-velocity stream must flow before OFFBOARD mode
/// and arming are commanded (PX4 rejects OFFBOARD without a stream).
const OFFBOARD_WARMUP: Duration = Duration::from_millis(300);
/// While streaming with no fresh stick frame, a keepalive setpoint is
/// emitted at this period so PX4's offboard-loss failsafe does not
/// trip between control frames.
const STREAM_KEEPALIVE: Duration = Duration::from_millis(50);
/// Message-interval requests are re-sent at this period until the
/// telemetry actually flows.
const INTERVAL_RETRY: Duration = Duration::from_secs(2);

/// MAV_CMD_DO_SET_MODE.
const CMD_DO_SET_MODE: u16 = 176;
/// MAV_CMD_SET_MESSAGE_INTERVAL.
const CMD_SET_MESSAGE_INTERVAL: u16 = 511;
/// MAV_MODE_FLAG_CUSTOM_MODE_ENABLED.
const BASE_MODE_CUSTOM: f32 = 1.0;
/// PX4 custom main mode OFFBOARD.
const PX4_MAIN_MODE_OFFBOARD: f32 = 6.0;
/// Message ids whose intervals are requested, with the interval in
/// microseconds: attitude quaternion and local position at ~30 Hz,
/// the estimator status at 5 Hz.
const STREAMS: [(u32, f32); 3] = [(31, 33_333.0), (32, 33_333.0), (230, 200_000.0)];

/// The uplink's time source; tests substitute a manually advanced
/// instant so sequencing is exercised without real-time sleeps.
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

/// Where the offboard arm sequence stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArmPhase {
    /// No arm requested; no stream.
    Idle,
    /// Zero-velocity stream flowing; OFFBOARD + arm pending warmup.
    Warmup,
    /// OFFBOARD and arm have been commanded; stream continues.
    Commanded,
}

/// The UDP MAVLink command uplink to PX4's onboard instance.
#[derive(Debug)]
pub struct Px4Uplink {
    socket: UdpSocket,
    target: SocketAddr,
    seq: u8,
    heading_sp_rad: f32,
    last_frame: Option<Instant>,
    last_vel_ned: [f32; 3],
    arm_phase: ArmPhase,
    warmup_since: Option<Instant>,
    last_heartbeat: Option<Instant>,
    last_interval_request: Option<Instant>,
    started: Instant,
    clock: UplinkClock,
    send_failures: u64,
    expected_system_id: u8,
    expected_component_id: u8,
}

impl Px4Uplink {
    /// Binds an ephemeral socket toward PX4's onboard command port
    /// (`PILOTAGE_PX4_FC_ADDR`, default `127.0.0.1:14580`).
    ///
    /// # Errors
    ///
    /// Returns the socket bind error.
    pub fn new() -> std::io::Result<Self> {
        let target = std::env::var("PILOTAGE_PX4_FC_ADDR")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 14_580)));
        let socket = UdpSocket::bind("127.0.0.1:0")?;
        socket.set_nonblocking(true)?;
        info!(%target, "PX4 offboard uplink ready");
        Ok(Self {
            socket,
            target,
            seq: 0,
            heading_sp_rad: 0.0,
            last_frame: None,
            last_vel_ned: [0.0; 3],
            arm_phase: ArmPhase::Idle,
            warmup_since: None,
            last_heartbeat: None,
            last_interval_request: None,
            started: Instant::now(),
            clock: UplinkClock::System,
            send_failures: 0,
            expected_system_id: 1,
            expected_component_id: 1,
        })
    }

    /// Selects the MAVLink system/component commands are addressed to.
    pub fn set_expected_source(&mut self, system_id: u8, component_id: u8) {
        self.expected_system_id = system_id;
        self.expected_component_id = component_id;
    }

    /// Retargets the uplink socket, for tests against a fake FC.
    pub fn set_target(&mut self, target: SocketAddr) {
        self.target = target;
    }

    /// Total refused sends, for enactment-truth counter deltas.
    #[must_use]
    pub fn send_failures(&self) -> u64 {
        self.send_failures
    }

    fn send(&mut self, frame: &[u8]) {
        if self.socket.send_to(frame, self.target).is_err() {
            self.send_failures = self.send_failures.wrapping_add(1);
            if self.send_failures == 1 || self.send_failures.is_multiple_of(100) {
                warn!(
                    failures = self.send_failures,
                    target = %self.target,
                    "PX4 uplink send failed"
                );
            }
        }
        self.seq = self.seq.wrapping_add(1);
    }

    fn time_boot_ms(&self) -> u32 {
        self.clock
            .now()
            .saturating_duration_since(self.started)
            .as_millis() as u32
    }

    /// Periodic presence and stream upkeep: the 1 Hz GCS heartbeat,
    /// message-interval requests until telemetry flows, keepalive
    /// setpoints between control frames, and the warmup step of the
    /// offboard arm sequence. Call at telemetry-sampling cadence.
    pub fn maintain(&mut self, telemetry_flowing: bool) {
        let now = self.clock.now();
        if self
            .last_heartbeat
            .is_none_or(|at| now.duration_since(at) >= HEARTBEAT_PERIOD)
        {
            self.last_heartbeat = Some(now);
            let frame = encode_gcs_heartbeat(self.seq);
            self.send(&frame);
        }
        if !telemetry_flowing
            && self
                .last_interval_request
                .is_none_or(|at| now.duration_since(at) >= INTERVAL_RETRY)
        {
            self.last_interval_request = Some(now);
            self.request_streams();
        }
        if self.arm_phase != ArmPhase::Idle
            && self
                .last_frame
                .is_none_or(|at| now.duration_since(at) >= STREAM_KEEPALIVE)
        {
            let velocity = self.last_vel_ned;
            self.send_velocity(velocity);
        }
        if self.arm_phase == ArmPhase::Warmup
            && self
                .warmup_since
                .is_some_and(|since| now.duration_since(since) >= OFFBOARD_WARMUP)
        {
            self.command_offboard_and_arm();
        }
    }

    fn request_streams(&mut self) {
        for (message_id, interval_us) in STREAMS {
            let frame = encode_command_long(
                self.seq,
                CMD_SET_MESSAGE_INTERVAL,
                [message_id as f32, interval_us, 0.0, 0.0, 0.0, 0.0, 0.0],
                self.expected_system_id,
                self.expected_component_id,
            );
            self.send(&frame);
        }
    }

    /// Starts the offboard arm sequence: seed the heading from the
    /// measured yaw and begin the zero-velocity stream; OFFBOARD mode
    /// and the arm command follow after the warmup.
    pub fn begin_arm(&mut self, current_yaw_rad: f32) {
        self.heading_sp_rad = current_yaw_rad;
        self.last_vel_ned = [0.0; 3];
        self.arm_phase = ArmPhase::Warmup;
        self.warmup_since = Some(self.clock.now());
        info!("offboard arm sequence: zero-velocity stream started");
        self.send_velocity([0.0; 3]);
    }

    fn command_offboard_and_arm(&mut self) {
        let mode = encode_command_long(
            self.seq,
            CMD_DO_SET_MODE,
            [
                BASE_MODE_CUSTOM,
                PX4_MAIN_MODE_OFFBOARD,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
            ],
            self.expected_system_id,
            self.expected_component_id,
        );
        self.send(&mode);
        let arm = encode_arm_command(
            self.seq,
            true,
            self.expected_system_id,
            self.expected_component_id,
        );
        self.send(&arm);
        self.arm_phase = ArmPhase::Commanded;
        info!("offboard arm sequence: OFFBOARD and arm commanded");
    }

    /// Disarms and stops the setpoint stream.
    pub fn send_disarm(&mut self) {
        let frame = encode_arm_command(
            self.seq,
            false,
            self.expected_system_id,
            self.expected_component_id,
        );
        self.send(&frame);
        self.arm_phase = ArmPhase::Idle;
        self.warmup_since = None;
        self.last_vel_ned = [0.0; 3];
        info!("disarm sent; setpoint stream stopped");
    }

    /// Converts one canonical stick frame (`[-1, 1]` roll/pitch/
    /// throttle/yaw; pitch + = forward, roll + = right, throttle + =
    /// climb, yaw + = clockwise) into a slew-limited velocity setpoint.
    /// Ignored while no arm sequence is active: PX4 rejects setpoints
    /// disarmed, and streaming before an explicit arm would mask the
    /// arm sequencing.
    pub fn send_stick_frame(&mut self, roll: f32, pitch: f32, throttle: f32, yaw: f32) {
        if self.arm_phase == ArmPhase::Idle {
            return;
        }
        let now = self.clock.now();
        let dt = self
            .last_frame
            .map_or(0.0, |t| now.duration_since(t).as_secs_f32())
            .clamp(0.0, MAX_DT_S);
        self.heading_sp_rad = wrap_pi(self.heading_sp_rad + yaw * MAX_YAW_RATE_RPS * dt);
        let heading = self.heading_sp_rad;
        let (sin, cos) = (heading.sin(), heading.cos());
        let fwd = pitch * MAX_HORIZONTAL_MPS;
        let lat = roll * MAX_HORIZONTAL_MPS;
        let demand = [
            fwd * cos - lat * sin,
            fwd * sin + lat * cos,
            -throttle * MAX_VERTICAL_MPS,
        ];
        let dv_max = MAX_ACCEL_MPS2 * dt.max(1.0 / 60.0);
        let mut slewed = [0.0_f32; 3];
        for (out, (target, current)) in slewed
            .iter_mut()
            .zip(demand.iter().zip(self.last_vel_ned.iter()))
        {
            *out = current + (target - current).clamp(-dv_max, dv_max);
        }
        self.send_velocity(slewed);
    }

    fn send_velocity(&mut self, vel_ned_mps: [f32; 3]) {
        self.last_vel_ned = vel_ned_mps;
        self.last_frame = Some(self.clock.now());
        let frame = encode_velocity_setpoint(
            self.seq,
            self.time_boot_ms(),
            vel_ned_mps,
            self.heading_sp_rad,
            self.expected_system_id,
            self.expected_component_id,
        );
        self.send(&frame);
    }

    /// Enacts link-loss neutralization: one final zero-velocity
    /// setpoint, then the stream stops so PX4's own offboard-loss
    /// failsafe takes the vehicle (Hold). Stopping the stream — not
    /// streaming neutral forever — is the honest enactment: a silent
    /// pilot must look silent to the FC too.
    pub fn neutralize(&mut self) {
        self.send_velocity([0.0; 3]);
        self.arm_phase = ArmPhase::Idle;
        self.warmup_since = None;
        info!("neutralized: stream stopped; PX4 offboard-loss failsafe takes over");
    }

    /// Whether an arm sequence is active (stream flowing), for tests.
    #[cfg(test)]
    pub(crate) fn streaming(&self) -> bool {
        self.arm_phase != ArmPhase::Idle
    }

    /// Switches to the manual clock, for tests.
    #[cfg(test)]
    pub(crate) fn use_manual_clock(&mut self) {
        self.clock = UplinkClock::Manual(Instant::now());
    }

    /// Advances the manual clock, for tests.
    #[cfg(test)]
    pub(crate) fn advance_clock(&mut self, by: Duration) {
        if let UplinkClock::Manual(at) = &mut self.clock {
            *at += by;
        }
    }
}

fn wrap_pi(angle: f32) -> f32 {
    let mut wrapped = angle;
    while wrapped > core::f32::consts::PI {
        wrapped -= 2.0 * core::f32::consts::PI;
    }
    while wrapped < -core::f32::consts::PI {
        wrapped += 2.0 * core::f32::consts::PI;
    }
    wrapped
}
