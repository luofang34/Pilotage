//! The uplink policy state and its arm, hold, and profile operations.

use std::time::Duration;

use pilotage_control_feel::{DemandEnvelope, ValidatedFlightFeelProfile};
use pilotage_mavlink::codec::{encode_arm_command, encode_velocity_setpoint};
use tracing::info;

mod control;
mod feel;
#[cfg(test)]
mod tests;

pub use control::{
    ActivationStep, AttitudeOutcome, DirectFrame, DirectRefusal, StickFrame, StickOutcome,
};

/// Longest believable gap between control frames when integrating the
/// yaw-rate stick; anything longer is a stall, not a dt.
pub const MAX_DT_S: f32 = 0.1;
/// Stick frames are suppressed this long after an arm/disarm send: the
/// FC stages inbound commands in a single slot, so a setpoint arriving
/// in the same poll batch would overwrite the arm before the control
/// loop consumes it.
const ARM_QUIET: Duration = Duration::from_millis(150);
/// How often a latched hold is repeated: well inside the FC's 500 ms
/// command timeout.
const HOLD_KEEPALIVE: Duration = Duration::from_millis(100);

/// The heading setpoint and whether a measurement seeded it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeadingState {
    /// The absolute heading setpoint, radians.
    pub heading_rad: f32,
    /// Whether a measured heading seeded the setpoint.
    pub valid: bool,
}

/// The flight uplink policy. Every time argument is a monotonic time
/// since an origin that the caller chooses once; the MAVLink boot time of
/// each frame is that time in milliseconds.
#[derive(Debug)]
pub struct UplinkPolicy {
    seq: u8,
    heading: HeadingState,
    last_frame: Option<Duration>,
    quiet_until: Option<Duration>,
    // Motors-idle gate: after arm, no velocity setpoints stream until
    // the first deliberate climb input. Streaming vz=0 to a grounded
    // vehicle commands "hold zero vertical velocity" at near-hover
    // thrust, which tips it over — real drones idle until the first
    // climb, so this does too.
    airborne: bool,
    // Brake-then-hold state: the captured hold point while every stick
    // is centered.
    hold_pos_ned: Option<[f32; 3]>,
    // When the link-loss hold last went out.
    last_hold: Option<Duration>,
    feel: feel::UplinkFeel,
    system_id: u8,
    component_id: u8,
}

impl UplinkPolicy {
    /// A policy with the control-feel `profile`, addressing the flight
    /// controller at MAVLink system 1, component 1.
    #[must_use]
    pub fn new(profile: ValidatedFlightFeelProfile) -> Self {
        Self {
            seq: 0,
            heading: HeadingState {
                heading_rad: 0.0,
                valid: false,
            },
            last_frame: None,
            quiet_until: None,
            airborne: false,
            hold_pos_ned: None,
            last_hold: None,
            feel: feel::UplinkFeel::new(profile),
            system_id: 1,
            component_id: 1,
        }
    }

    /// Selects the MAVLink system and component that the frames address.
    pub fn set_target(&mut self, system_id: u8, component_id: u8) {
        self.system_id = system_id;
        self.component_id = component_id;
    }

    /// The MAVLink system and component that the frames address.
    #[must_use]
    pub const fn target(&self) -> (u8, u8) {
        (self.system_id, self.component_id)
    }

    /// The control-feel envelope of the active profile.
    #[must_use]
    pub fn envelope(&self) -> DemandEnvelope {
        self.feel.envelope()
    }

    /// The active control-feel profile.
    #[must_use]
    pub fn profile(&self) -> &ValidatedFlightFeelProfile {
        self.feel.validated_profile()
    }

    /// Installs a new control-feel profile and clears the temporal state.
    pub fn install_profile(&mut self, profile: ValidatedFlightFeelProfile) {
        self.reset_temporal_state();
        self.feel.install(profile);
    }

    /// The heading setpoint state.
    #[must_use]
    pub const fn heading(&self) -> HeadingState {
        self.heading
    }

    /// Puts back a heading state, after a frame that carried a new
    /// heading did not reach the flight controller.
    pub fn restore_heading(&mut self, heading: HeadingState) {
        self.heading = heading;
    }

    /// Whether the setpoint stream is open: a climb demand opened it
    /// after the last arm or reset.
    #[must_use]
    pub const fn airborne(&self) -> bool {
        self.airborne
    }

    /// Opens the setpoint stream without a climb demand, for a rig that
    /// starts in direct control.
    pub fn open_setpoint_stream(&mut self) {
        self.airborne = true;
    }

    /// The arm command frame. It seeds the heading from the measured yaw
    /// and starts the quiet interval.
    pub fn arm(&mut self, now: Duration, current_yaw_rad: f32) -> Vec<u8> {
        self.heading = HeadingState {
            heading_rad: if current_yaw_rad.is_finite() {
                current_yaw_rad
            } else {
                0.0
            },
            valid: current_yaw_rad.is_finite(),
        };
        self.arm_command(now, true)
    }

    /// The disarm command frame. It needs no measurement.
    pub fn disarm(&mut self, now: Duration) -> Vec<u8> {
        self.arm_command(now, false)
    }

    fn arm_command(&mut self, now: Duration, arm: bool) -> Vec<u8> {
        self.reset_temporal_state();
        self.quiet_until = Some(now + ARM_QUIET);
        self.airborne = false;
        let frame = encode_arm_command(self.seq, arm, self.system_id, self.component_id);
        self.seq = self.seq.wrapping_add(1);
        info!(arm, "arm command frame");
        frame.to_vec()
    }

    /// A zero-velocity setpoint that holds the current heading: the
    /// link-loss neutralize action. The flight controller's velocity mode
    /// brakes to a hover on zero demand.
    pub fn neutral(&mut self, now: Duration) -> Vec<u8> {
        self.last_hold = Some(now);
        self.velocity_frame(now, [0.0; 3])
    }

    /// The hold again when the last one is older than the keep-alive
    /// interval. The flight controller treats a command older than its
    /// timeout as a lost uplink and descends, so the hold has to keep
    /// arriving while a latch holds the vehicle. A vehicle that has not
    /// taken off gets nothing.
    pub fn keep_holding(&mut self, now: Duration) -> Option<Vec<u8>> {
        if !self.airborne {
            return None;
        }
        let due = self
            .last_hold
            .is_none_or(|at| now.saturating_sub(at) >= HOLD_KEEPALIVE);
        due.then(|| self.neutral(now))
    }

    /// Invalidates a captured hold point. A link-loss transition must call
    /// this: the hold point was captured under the lost lease, and the
    /// vehicle may have drifted while neutralized.
    pub fn clear_hold_state(&mut self) {
        self.reset_temporal_state();
    }

    /// Clears all command state after a vehicle or simulator reset.
    pub fn reset_for_vehicle_reset(&mut self) {
        self.reset_temporal_state();
        self.airborne = false;
        self.quiet_until = None;
        self.heading.valid = false;
    }

    fn reset_temporal_state(&mut self) {
        self.last_frame = None;
        self.hold_pos_ned = None;
        self.feel.reset();
    }

    fn velocity_frame(&mut self, now: Duration, velocity_ned_mps: [f32; 3]) -> Vec<u8> {
        let frame = encode_velocity_setpoint(
            self.seq,
            boot_ms(now),
            velocity_ned_mps,
            self.heading.heading_rad,
            self.system_id,
            self.component_id,
        );
        self.seq = self.seq.wrapping_add(1);
        frame.to_vec()
    }

    /// Whether `now` is inside the post-arm quiet interval.
    fn in_quiet_interval(&mut self, now: Duration) -> bool {
        if let Some(quiet) = self.quiet_until {
            if now < quiet {
                return true;
            }
            self.quiet_until = None;
        }
        false
    }
}

/// The MAVLink boot time of a frame sent at `now`. It wraps after 49 days,
/// as the field does.
fn boot_ms(now: Duration) -> u32 {
    (now.as_millis() & u128::from(u32::MAX)) as u32
}

/// Hooks for the tests of crates that wrap the policy.
#[cfg(any(test, feature = "test-support"))]
impl UplinkPolicy {
    /// The captured hold point, if any.
    #[must_use]
    pub const fn hold_point(&self) -> Option<[f32; 3]> {
        self.hold_pos_ned
    }

    /// Plants a captured hold point.
    pub fn seed_hold(&mut self, pos_ned_m: [f32; 3]) {
        self.hold_pos_ned = Some(pos_ned_m);
    }

    /// Ends the post-arm quiet interval now.
    pub fn expire_quiet(&mut self) {
        self.quiet_until = None;
    }
}
