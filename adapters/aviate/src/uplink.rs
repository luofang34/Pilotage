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

use pilotage_control_feel::{FeelDigest, FlightFeelProfile, ValidatedFlightFeelProfile};
use pilotage_mavlink::codec::{encode_arm_command, encode_velocity_setpoint};

mod control;
mod direct;
mod fc_replies;
mod feel;
#[cfg(test)]
mod tests;

pub use direct::{
    ATTITUDE_SETPOINT_FRAME_BYTES, ExactDirectError, ExactDirectSetpoint, SimulatorDirectAuthority,
    TransmittedDirectSetpoint,
};

/// Longest believable gap between control frames when integrating the
/// yaw-rate stick; anything longer is a stall, not a dt.
pub(crate) const MAX_DT_S: f32 = 0.1;
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
    heading_valid: bool,
    last_frame: Option<Instant>,
    quiet_until: Option<Instant>,
    // Motors-idle gate: after arm, no velocity setpoints stream until
    // the first deliberate climb input. Streaming vz=0 to a grounded
    // vehicle commands "hold zero vertical velocity" at near-hover
    // thrust, which tips it over — real drones idle until the first
    // climb, so this does too.
    airborne: bool,
    // Brake-then-hold state: the captured hold point while every stick
    // is centered.
    hold_pos_ned: Option<[f32; 3]>,
    feel: feel::UplinkFeel,
    started: Instant,
    clock: UplinkClock,
    send_failures: u64,
    expected_system_id: u8,
    expected_component_id: u8,
}

impl FlightUplink {
    /// Binds an ephemeral socket toward the FC's command port
    /// (`PILOTAGE_AVIATE_FC_ADDR`, default `127.0.0.1:20000` — the SITL
    /// FC's MAVLink/GCS port).
    ///
    /// # Errors
    ///
    /// Returns an error if the default profile or socket cannot initialize.
    pub fn new() -> std::io::Result<Self> {
        let profile = ValidatedFlightFeelProfile::new(FlightFeelProfile::legacy_compatibility())
            .map_err(std::io::Error::other)?;
        Self::new_with_profile(profile)
    }

    /// Binds an ephemeral socket with a validated control-feel profile.
    ///
    /// # Errors
    ///
    /// Returns an error if the profile does not match Alia or the socket
    /// cannot initialize.
    pub(crate) fn new_with_profile(profile: ValidatedFlightFeelProfile) -> std::io::Result<Self> {
        Self::bind_with_profile(profile, default_command_endpoint())
    }

    /// Binds an ephemeral socket toward an explicit FC command endpoint.
    ///
    /// A tuning trial names the endpoint it commands, because that endpoint
    /// is part of the direct-transport identity. Reading it from the ambient
    /// environment would leave the identity unbound.
    ///
    /// # Errors
    ///
    /// Returns an error if the default profile or socket cannot initialize.
    pub fn bind_to(command_endpoint: SocketAddr) -> std::io::Result<Self> {
        let profile = ValidatedFlightFeelProfile::new(FlightFeelProfile::legacy_compatibility())
            .map_err(std::io::Error::other)?;
        Self::bind_with_profile(profile, command_endpoint)
    }

    fn bind_with_profile(
        profile: ValidatedFlightFeelProfile,
        target: SocketAddr,
    ) -> std::io::Result<Self> {
        crate::adapter::validate_aviate_profile_bindings(&profile)
            .map_err(std::io::Error::other)?;
        let feel_digest = FeelDigest::calculate(&profile).map_err(std::io::Error::other)?;
        let socket = UdpSocket::bind("127.0.0.1:0")?;
        socket.set_nonblocking(true)?;
        info!(
            %target,
            feel_profile_id = %profile.profile().profile_id,
            feel_schema = profile.profile().schema_version,
            feel_digest = %feel_digest,
            "Aviate flight uplink ready"
        );
        Ok(Self {
            socket,
            target,
            seq: 0,
            heading_sp_rad: 0.0,
            heading_valid: false,
            last_frame: None,
            quiet_until: None,
            airborne: false,
            hold_pos_ned: None,
            feel: feel::UplinkFeel::new(profile),
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

    pub(crate) fn monotonic_now(&self) -> Instant {
        self.clock.now()
    }

    /// The FC command endpoint every uplink frame is addressed to.
    pub const fn command_endpoint(&self) -> SocketAddr {
        self.target
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
        self.heading_sp_rad = if current_yaw_rad.is_finite() {
            current_yaw_rad
        } else {
            0.0
        };
        self.heading_valid = current_yaw_rad.is_finite();
        self.send_arm_command(true);
    }

    /// Disarms without requiring a measurement that may have failed.
    pub fn send_disarm(&mut self) {
        self.send_arm_command(false);
    }

    fn send_arm_command(&mut self, arm: bool) {
        self.reset_temporal_state();
        self.quiet_until = Some(self.clock.now() + ARM_QUIET);
        self.airborne = false;
        let frame = encode_arm_command(
            self.seq,
            arm,
            self.expected_system_id,
            self.expected_component_id,
        );
        self.send(&frame);
        info!(arm, "sent arm command to FC");
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
        self.reset_temporal_state();
    }

    /// Clears all command state after a vehicle or simulator reset.
    pub fn reset_for_vehicle_reset(&mut self) {
        self.reset_temporal_state();
        self.airborne = false;
        self.quiet_until = None;
        self.heading_valid = false;
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

    /// Returns the heading state for activation rollback tests.
    #[cfg(test)]
    pub(crate) const fn heading_state_for_test(&self) -> (f32, bool) {
        (self.heading_sp_rad, self.heading_valid)
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

    pub(crate) fn envelope(&self) -> pilotage_control_feel::DemandEnvelope {
        self.feel.envelope()
    }

    pub(crate) fn install_profile(&mut self, profile: ValidatedFlightFeelProfile) {
        self.reset_temporal_state();
        self.feel.install(profile);
    }

    #[cfg(test)]
    pub(crate) fn active_profile_for_test(&self) -> &ValidatedFlightFeelProfile {
        self.feel.validated_profile()
    }

    fn reset_temporal_state(&mut self) {
        self.last_frame = None;
        self.hold_pos_ned = None;
        self.feel.reset();
    }
}

fn default_command_endpoint() -> SocketAddr {
    std::env::var("PILOTAGE_AVIATE_FC_ADDR")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 20000)))
}
