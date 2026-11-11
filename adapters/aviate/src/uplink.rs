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
//!
//! The sans-IO `pilotage_flight_uplink::UplinkPolicy` decides every
//! frame. This module adds the transport (a UDP socket, or in-memory
//! queues for an embedding host), the monotonic clock, and the count of
//! refused sends.

use std::net::SocketAddr;
#[cfg(feature = "net")]
use std::net::UdpSocket;

use crate::clock::AdapterClock;
use std::time::Duration;
use web_time::Instant;

use tracing::{info, warn};

use pilotage_control_feel::ValidatedFlightFeelProfile;
#[cfg(feature = "net")]
use pilotage_control_feel::{FeelDigest, FlightFeelProfile};
use pilotage_flight_uplink::{ActivationStep, StickFrame, UplinkPolicy};

mod direct;
mod fc_replies;
mod transport;
use transport::UplinkTransport;
#[cfg(test)]
mod tests;

pub use direct::{
    ATTITUDE_SETPOINT_FRAME_BYTES, ExactDirectError, ExactDirectSetpoint, SimulatorDirectAuthority,
    TransmittedDirectSetpoint,
};

/// The MAVLink command uplink to the FC: the transport and the clock
/// around the sans-IO [`UplinkPolicy`], which decides every frame.
#[derive(Debug)]
pub struct FlightUplink {
    transport: UplinkTransport,
    target: SocketAddr,
    policy: UplinkPolicy,
    started: Instant,
    clock: AdapterClock,
    send_failures: u64,
    /// The arm (true) or disarm (false) last sent, until its ack arrives.
    arm_sent: Option<bool>,
    /// The FC's answer to the arm or disarm last sent.
    arm_ack: Option<pilotage_adapter_api::FcCommandAck>,
}

impl FlightUplink {
    /// Binds an ephemeral socket toward the FC's command port
    /// (`PILOTAGE_AVIATE_FC_ADDR`, default `127.0.0.1:20000` — the SITL
    /// FC's MAVLink/GCS port).
    ///
    /// # Errors
    ///
    /// Returns an error if the default profile or socket cannot initialize.
    #[cfg(feature = "net")]
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
    #[cfg(feature = "net")]
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
    #[cfg(feature = "net")]
    pub fn bind_to(command_endpoint: SocketAddr) -> std::io::Result<Self> {
        let profile = ValidatedFlightFeelProfile::new(FlightFeelProfile::legacy_compatibility())
            .map_err(std::io::Error::other)?;
        Self::bind_with_profile(profile, command_endpoint)
    }

    #[cfg(feature = "net")]
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
        Ok(Self::with_transport(
            UplinkTransport::Udp(socket),
            target,
            profile,
            AdapterClock::System,
            Instant::now(),
        ))
    }

    /// An uplink whose frames and FC replies stay in memory for an
    /// embedding host to move, on a manual clock that starts at `now`.
    ///
    /// # Errors
    ///
    /// Returns an error if the profile does not match Alia.
    pub fn in_memory(profile: ValidatedFlightFeelProfile, now: Instant) -> std::io::Result<Self> {
        crate::adapter::validate_aviate_profile_bindings(&profile)
            .map_err(std::io::Error::other)?;
        Ok(Self::with_transport(
            UplinkTransport::memory(),
            SocketAddr::from(([0, 0, 0, 0], 0)),
            profile,
            AdapterClock::Manual(now),
            now,
        ))
    }

    fn with_transport(
        transport: UplinkTransport,
        target: SocketAddr,
        profile: ValidatedFlightFeelProfile,
        clock: AdapterClock,
        started: Instant,
    ) -> Self {
        Self {
            transport,
            target,
            policy: UplinkPolicy::new(profile),
            started,
            clock,
            send_failures: 0,
            arm_sent: None,
            arm_ack: None,
        }
    }

    /// Sets the time of a manual clock; a system clock does not change.
    pub fn set_now(&mut self, now: Instant) {
        self.clock.set(now);
    }

    /// Queues one FC reply datagram for the next [`Self::poll_fc`]; false
    /// for a socket uplink, which receives its own replies.
    pub fn push_fc_reply(&mut self, datagram: &[u8]) -> bool {
        self.transport.push_reply(datagram)
    }

    /// Takes the frames an in-memory uplink sent since the last call, in
    /// send order.
    pub fn take_sent_frames(&mut self) -> Vec<Vec<u8>> {
        self.transport.take_sent()
    }

    /// Switches to the manually advanced clock, anchored at construction
    /// time, so tests drive the quiet window and slew dt deterministically.
    #[cfg(test)]
    pub(crate) fn use_manual_clock(&mut self) {
        self.clock = AdapterClock::Manual(self.started);
    }

    /// Advances the manual clock; a no-op on the system clock.
    #[cfg(test)]
    pub(crate) fn advance_clock(&mut self, dt: Duration) {
        if let AdapterClock::Manual(at) = &mut self.clock {
            *at += dt;
        }
    }

    pub(crate) fn monotonic_now(&self) -> Instant {
        self.clock.now()
    }

    /// The policy time: the monotonic time since the uplink started.
    fn elapsed(&self) -> Duration {
        self.clock.now().saturating_duration_since(self.started)
    }

    /// The FC command endpoint every uplink frame is addressed to.
    pub const fn command_endpoint(&self) -> SocketAddr {
        self.target
    }

    /// Selects the MAVLink system/component whose replies may affect state.
    pub fn set_expected_source(&mut self, system_id: u8, component_id: u8) {
        self.policy.set_target(system_id, component_id);
    }

    /// Sends one frame. A refused send counts, and the count is logged
    /// on the first refusal and on every hundredth.
    fn send(&mut self, frame: &[u8]) {
        if !self.transport.send(frame, self.target) {
            self.send_failures = self.send_failures.wrapping_add(1);
            if self.send_failures == 1 || self.send_failures.is_multiple_of(100) {
                warn!(
                    failures = self.send_failures,
                    target = %self.target,
                    "flight uplink send failed"
                );
            }
        }
    }

    /// Arms and re-seeds heading from the measured yaw.
    pub fn send_arm(&mut self, current_yaw_rad: f32) {
        let frame = self.policy.arm(self.elapsed(), current_yaw_rad);
        self.arm_command_sent(true);
        self.send(&frame);
        info!(arm = true, "sent arm command to FC");
    }

    /// Disarms without requiring a measurement that may have failed.
    pub fn send_disarm(&mut self) {
        let frame = self.policy.disarm(self.elapsed());
        self.arm_command_sent(false);
        self.send(&frame);
        info!(arm = false, "sent arm command to FC");
    }

    /// A new arm or disarm: the answer to the previous one no longer
    /// describes the command in flight.
    fn arm_command_sent(&mut self, arm: bool) {
        self.arm_sent = Some(arm);
        self.arm_ack = None;
    }

    /// The FC's answer to the arm or disarm last sent, once its
    /// COMMAND_ACK arrived.
    pub fn arm_ack(&self) -> Option<pilotage_adapter_api::FcCommandAck> {
        self.arm_ack
    }

    /// Sends a zero-velocity setpoint holding the current heading — the
    /// link-loss neutralize action (the FC's velocity mode brakes to a
    /// hover on zero demand).
    pub fn send_neutral(&mut self) {
        let frame = self.policy.neutral(self.elapsed());
        self.send(&frame);
    }

    /// Repeats the link-loss hold while a latch holds the vehicle; see
    /// [`UplinkPolicy::keep_holding`].
    pub(crate) fn keep_holding(&mut self) {
        if let Some(frame) = self.policy.keep_holding(self.elapsed()) {
            self.send(&frame);
        }
    }

    /// Prepares one neutral control-feel activation.
    ///
    /// `Ok(false)` means that the active command sequence is not ready.
    /// `Ok(true)` means that the adapter can commit the candidate.
    pub(crate) fn prepare_control_feel_activation(
        &mut self,
        current_yaw_rad: f32,
    ) -> Result<bool, &'static str> {
        match self
            .policy
            .prepare_control_feel_activation(self.elapsed(), current_yaw_rad)?
        {
            ActivationStep::NotReady => Ok(false),
            ActivationStep::Ready => Ok(true),
            ActivationStep::Send { frame, prior } => {
                let failures_before = self.send_failures;
                self.send(&frame);
                if self.send_failures != failures_before {
                    self.policy.restore_heading(prior);
                    return Err("the neutral activation setpoint was not sent");
                }
                Ok(true)
            }
        }
    }

    /// Sends one direct attitude and thrust setpoint.
    pub fn send_attitude_frame(
        &mut self,
        roll_rad: f32,
        pitch_rad: f32,
        yaw_rad: f32,
        thrust_norm: f32,
    ) -> bool {
        let heading = self.policy.heading().heading_rad;
        self.send_attitude_frame_seeded(
            roll_rad,
            pitch_rad,
            yaw_rad,
            thrust_norm,
            [0.0, 0.0, heading],
        )
    }

    /// Sends one direct setpoint and seeds a new direct mode from measured attitude.
    pub(crate) fn send_attitude_frame_seeded(
        &mut self,
        roll_rad: f32,
        pitch_rad: f32,
        yaw_rad: f32,
        thrust_norm: f32,
        seed_attitude_rad: [f32; 3],
    ) -> bool {
        let outcome = self.policy.attitude_frame(
            self.elapsed(),
            [roll_rad, pitch_rad, yaw_rad],
            thrust_norm,
            seed_attitude_rad,
        );
        if let Some(frame) = outcome.frame {
            self.send(&frame);
        }
        outcome.constrained
    }

    /// Shapes one normalized stick frame and sends a velocity or hold setpoint.
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
        current_accel_ned_mps2: Option<[f32; 3]>,
    ) -> bool {
        let stick = StickFrame {
            roll,
            pitch,
            throttle,
            yaw,
            current_yaw_rad,
            current_pos_ned_m,
            current_vel_ned_mps,
            current_accel_ned_mps2,
        };
        let outcome = self.policy.stick_frame(self.elapsed(), &stick);
        if let Some(frame) = outcome.frame {
            self.send(&frame);
        }
        outcome.constrained
    }

    /// The MAVLink (system, component) identity this uplink accepts FC
    /// reports from — the provenance identity for those reports.
    pub fn expected_source(&self) -> (u8, u8) {
        self.policy.target()
    }

    /// Wrapping count of datagrams the socket refused to send. A safety
    /// enactment (link-loss neutralize) compares this across its send:
    /// an increment means the FC never received the command, which must
    /// surface as a typed enactment failure — never as silent success.
    pub fn send_failures(&self) -> u64 {
        self.send_failures
    }

    /// Invalidates any captured position-hold context; see
    /// [`UplinkPolicy::clear_hold_state`].
    pub fn clear_hold_state(&mut self) {
        self.policy.clear_hold_state();
    }

    /// Clears all command state after a vehicle or simulator reset.
    pub fn reset_for_vehicle_reset(&mut self) {
        self.policy.reset_for_vehicle_reset();
    }

    /// Whether a position-hold point is currently captured, for tests.
    #[cfg(test)]
    pub(crate) fn hold_captured(&self) -> bool {
        self.policy.hold_point().is_some()
    }

    /// Plants a captured hold point, for tests exercising the stale-hold
    /// invalidation contract without flying a full trajectory.
    #[cfg(test)]
    pub(crate) fn seed_hold_for_test(&mut self, pos_ned_m: [f32; 3]) {
        self.policy.seed_hold(pos_ned_m);
    }

    /// Returns the heading state for activation rollback tests.
    #[cfg(test)]
    pub(crate) const fn heading_state_for_test(&self) -> (f32, bool) {
        let heading = self.policy.heading();
        (heading.heading_rad, heading.valid)
    }

    /// Expires the post-arm quiet window immediately, so tests advance
    /// past it deterministically instead of sleeping wall-clock time.
    #[cfg(test)]
    pub(crate) fn expire_quiet_for_test(&mut self) {
        self.policy.expire_quiet();
    }

    /// The local socket address this uplink receives FC replies on.
    ///
    /// # Errors
    ///
    /// Returns the socket introspection error.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.transport.local_addr()
    }

    /// Overrides the target, for tests.
    #[cfg(test)]
    pub(crate) fn set_target(&mut self, target: SocketAddr) {
        self.target = target;
    }

    pub(crate) fn envelope(&self) -> pilotage_control_feel::DemandEnvelope {
        self.policy.envelope()
    }

    pub(crate) fn install_profile(&mut self, profile: ValidatedFlightFeelProfile) {
        self.policy.install_profile(profile);
    }

    #[cfg(test)]
    pub(crate) fn active_profile_for_test(&self) -> &ValidatedFlightFeelProfile {
        self.policy.profile()
    }
}

#[cfg(feature = "net")]
fn default_command_endpoint() -> SocketAddr {
    std::env::var("PILOTAGE_AVIATE_FC_ADDR")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 20000)))
}
