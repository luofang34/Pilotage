//! Direct flight: body rates or attitude with collective thrust, as
//! GUIDED SET_ATTITUDE_TARGET.
//!
//! GUIDED rather than ACRO with RC override: SET_ATTITUDE_TARGET carries
//! rates in rad/s and thrust as numbers the FC applies the tick it
//! receives them, under the same GUIDED timeout and the same mode the
//! velocity family uses, so a family switch never changes the FC mode.
//! RC override would pass through stick scaling, expo, and the RC input
//! filter, and switching mode to ACRO and back would reset the
//! controllers on every switch.
//!
//! The canonical thrust puts hover at 0.5 (the same midpoint the command
//! gate treats as neutral). ArduCopter's own manual throttle curve does
//! the same: below the midpoint thrust scales linearly to zero, above it
//! linearly to full, with the FC's hover thrust (MOT_THST_HOVER) at the
//! midpoint. A switch into direct flight starts from hover thrust and
//! slews, so no family switch steps the thrust.

use std::time::Instant;

use pilotage_mavlink::codec::AttitudeTargetKind;
use pilotage_mavlink::{AttitudeThrustDemand, OffboardSetpoint};
use tracing::info;

use super::{ArduPilotUplink, FlightPhase};

/// Full-envelope direct body rate about any axis, rad/s (~170°/s).
pub const MAX_BODY_RATE_RPS: f32 = 3.0;
/// The tilt bound for attitude setpoints, rad (ArduCopter's default
/// ANGLE_MAX is 30°; this stays inside it).
pub const MAX_DIRECT_TILT_RAD: f32 = 0.5;
/// The fastest the FC thrust may change, per second of the full range.
const MAX_THRUST_SLEW_PER_S: f32 = 1.5;
/// Longest believable gap between direct frames when slewing.
const MAX_SLEW_DT_S: f32 = 0.1;

/// What a direct intent did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectOutcome {
    /// A setpoint was published; `slewed` when its thrust was held back
    /// from the demand by the slew limit.
    Streamed { slewed: bool },
    /// Landing: the FC owns the descent.
    Landing,
    /// An unresolved takeoff procedure blocks direct control.
    TakeoffBlocked,
    /// The FC reports itself disarmed.
    NotArmed,
    /// The lane to the link is gone.
    LaneClosed,
}

/// Maps the canonical thrust (hover at 0.5) onto the FC's thrust scale
/// with hover thrust `hover`.
pub(crate) fn fc_thrust(canonical: f32, hover: f32) -> f32 {
    let canonical = if canonical.is_nan() {
        0.5
    } else {
        canonical.clamp(0.0, 1.0)
    };
    if canonical <= 0.5 {
        canonical / 0.5 * hover
    } else {
        hover + (canonical - 0.5) / 0.5 * (1.0 - hover)
    }
}

impl ArduPilotUplink {
    /// Applies one direct intent. `hover` is the FC's hover thrust.
    pub(crate) fn apply_direct(
        &mut self,
        kind: AttitudeTargetKind,
        canonical_thrust: f32,
        armed: bool,
        hover: f32,
        now: Instant,
    ) -> DirectOutcome {
        self.observe_armed(armed);
        if !armed {
            return DirectOutcome::NotArmed;
        }
        if self.takeoff_blocks_motion() {
            return DirectOutcome::TakeoffBlocked;
        }
        if self.phase == FlightPhase::Landing {
            return DirectOutcome::Landing;
        }
        if self.phase != FlightPhase::Flying {
            info!("direct flight: the FC follows thrust from here, on the ground or not");
            self.phase = FlightPhase::Flying;
        }
        let demand = fc_thrust(canonical_thrust, hover);
        let (from, dt) = match self.direct_thrust {
            Some((last, at)) => (
                last,
                now.duration_since(at)
                    .as_secs_f32()
                    .clamp(1.0 / 60.0, MAX_SLEW_DT_S),
            ),
            None => (hover, 1.0 / 60.0),
        };
        let step = MAX_THRUST_SLEW_PER_S * dt;
        let thrust = from + (demand - from).clamp(-step, step);
        self.direct_thrust = Some((thrust, now));
        let published = self.velocity.send(Some(OffboardSetpoint::AttitudeThrust(
            AttitudeThrustDemand {
                kind,
                thrust,
                target_system: self.system_id,
                target_component: self.component_id,
            },
        )));
        if published.is_err() {
            self.send_failures = self.send_failures.wrapping_add(1);
            return DirectOutcome::LaneClosed;
        }
        DirectOutcome::Streamed {
            slewed: (thrust - demand).abs() > f32::EPSILON,
        }
    }
}

#[cfg(test)]
mod tests;
