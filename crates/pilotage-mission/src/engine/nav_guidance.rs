//! Display-facing navigation guidance: the plan-relative geometry the
//! executor is flying, in canonical units.
//!
//! This is display context (ADR-0031, ADR-0024): never an input to
//! control validation and never a stand-in for a missing estimate.
//! Absence is meaningful — an executor flying nothing has no guidance,
//! and consumers remove the deviation display rather than centering it.
//! Scaling deviations to instrument dots is display policy and happens
//! past this boundary (ADR-0017).

use navigate_contract::{AltitudeConstraint, GeodeticPosition, LateralReference, SolutionQuality};
use navigate_geodesy::{cross_track_from_course_m, cross_track_m, distance_m, initial_bearing_rad};
use navigate_route::{HoldStage, HoldStatus, Leg};

use crate::body::GEODESY;

use super::{
    FlightPhase, MissionEngine, MissionState, PositionIntegrity, PositionKind, SteeringIntegrity,
    ToFrom,
};

#[cfg(test)]
mod tests;

/// Quality of the navigation solution the guidance geometry rests on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NavQuality {
    /// Within the filter's accuracy bounds and admission health.
    Good,
    /// Usable with caution: bounds exceeded or sources degraded.
    Degraded,
    /// Not usable for any navigation decision on the navigator's own
    /// grade. Whether it is flown all the same is the engine's judgment
    /// ([`super::SteeringIntegrity`]), which a display follows.
    Unusable,
}

/// A solution quality this vocabulary cannot read is treated as
/// [`NavQuality::Unusable`]: the upstream classification grows in the
/// contract crate first, and an unreadable grade is not a good one.
impl From<SolutionQuality> for NavQuality {
    fn from(quality: SolutionQuality) -> Self {
        match quality {
            SolutionQuality::Good => Self::Good,
            SolutionQuality::Degraded => Self::Degraded,
            _ => Self::Unusable,
        }
    }
}

/// One guidance sample: the active leg's geometry relative to the
/// current navigation solution.
#[derive(Debug, Clone, PartialEq)]
pub struct NavGuidance {
    /// Identifier of the waypoint being flown toward.
    pub to_ident: String,
    /// Identifier of the leg's origin fix; `None` on a direct-to leg or
    /// holding circuit, which has no upstream fix.
    pub from_ident: Option<String>,
    /// Desired track, radians clockwise from true north in `[0, 2π)`. On
    /// a direct-to leg this is the live bearing to the waypoint.
    pub course_rad: f64,
    /// Cross-track deviation in meters, positive right of course.
    /// `None` when no lateral course is being tracked: a direct-to leg —
    /// the climb and the initial leg — anchors its track at ownship and
    /// has no cross-track geometry, as does a leg whose endpoints sit too
    /// close together to define a direction.
    pub lateral_deviation_m: Option<f64>,
    /// Deviation from the vertical profile in meters, positive above the
    /// profile; `None` when the active waypoint carries no altitude
    /// constraint.
    pub vertical_deviation_m: Option<f64>,
    /// Great-circle distance to the active waypoint, meters.
    pub distance_to_waypoint_m: f64,
    /// Index of the active leg's destination waypoint in fly order.
    pub leg_index: u32,
    /// Waypoints in the plan being flown.
    pub waypoint_count: u32,
    /// The engine's judgment of the solution this geometry rests on. A
    /// display keeps the guidance while a solution is flown, within its
    /// limits or by dead reckoning.
    pub integrity: SteeringIntegrity,
    /// Whether the active waypoint is ahead of (`To`) or behind (`From`)
    /// the vehicle along the desired course; `None` while no lateral
    /// course is tracked, which removes the deviation bar.
    pub to_from: Option<ToFrom>,
    /// The vehicle flies a holding pattern instead of sequencing legs:
    /// the geometry is the inbound leg to the holding fix, whose TO/FROM
    /// flips every circuit.
    pub holding: bool,
    /// The kind of position this geometry rests on; `None` when the
    /// engine cannot name one, which removes the display.
    pub position: Option<PositionKind>,
    /// The engine's judgment of that position; `None` when it has none an
    /// instrument can present, which removes the display.
    pub position_integrity: Option<PositionIntegrity>,
    /// Flight phase of the active leg (`Enroute`, `Terminal`, `Approach`,
    /// or `Missed`).
    pub phase: FlightPhase,
    /// Effective Required Navigation Performance of the active leg, metres.
    pub rnp_m: f64,
}

impl MissionEngine {
    /// The guidance the executor is flying, or `None` when it is flying
    /// nothing: before a first solution, while arming, and once the plan
    /// is complete with no holding pattern flown after it. In a holding
    /// pattern it is the inbound leg to the holding fix. The geometry
    /// comes from the solution and active leg the latest
    /// [`MissionEngine::tick`] left behind, so this reads state and never
    /// re-runs fusion or sequencing.
    #[must_use]
    pub fn nav_guidance(&self) -> Option<NavGuidance> {
        let solution = self.last_solution.as_ref()?;
        let phase = self.flight_phase();
        let rnp_m = self.rnp_m();
        match self.state() {
            MissionState::AfterPlan => {
                let leg = self.holding_pattern_leg(solution)?;
                let waypoint_count = self.route.waypoints().len();
                let steering = self.steering_source();
                Some(NavGuidance {
                    to_ident: leg.to.ident,
                    from_ident: None,
                    course_rad: leg.desired_track_rad,
                    lateral_deviation_m: leg.cross_track_m,
                    vertical_deviation_m: leg.vertical_deviation_m,
                    distance_to_waypoint_m: leg.distance_to_go_m,
                    leg_index: u32::try_from(waypoint_count.saturating_sub(1)).unwrap_or(u32::MAX),
                    waypoint_count: u32::try_from(waypoint_count).unwrap_or(u32::MAX),
                    integrity: steering.integrity,
                    to_from: Some(leg.to_from),
                    holding: true,
                    position: steering.position_kind(),
                    position_integrity: steering.position_integrity(),
                    phase,
                    rnp_m,
                })
            }
            MissionState::AwaitSolution
            | MissionState::Arming
            | MissionState::Held
            | MissionState::Complete
            | MissionState::Failed => None,
            MissionState::Climb | MissionState::Enroute => {
                let leg = self.route.active_leg()?;
                let steering = self.steering_source();
                let mut guidance = leg_guidance(
                    &solution.position,
                    &leg,
                    self.route.hold_status(),
                    self.route.waypoints().len(),
                    steering.integrity,
                    phase,
                    rnp_m,
                );
                guidance.position = steering.position_kind();
                guidance.position_integrity = steering.position_integrity();
                Some(guidance)
            }
        }
    }
}

/// The plan-relative geometry of one active leg from `position`.
fn leg_guidance(
    position: &GeodeticPosition,
    leg: &Leg<'_>,
    hold_status: Option<HoldStatus>,
    waypoint_count: usize,
    integrity: SteeringIntegrity,
    phase: FlightPhase,
    rnp_m: f64,
) -> NavGuidance {
    let to = leg.to;
    let (from_ident, course_rad, lateral_deviation_m, holding) =
        leg_lateral_geometry(position, leg, hold_status);
    let distance_to_waypoint_m = distance_m(position, &to.position, GEODESY);
    NavGuidance {
        to_ident: to.ident.as_str().to_owned(),
        from_ident,
        course_rad,
        lateral_deviation_m,
        vertical_deviation_m: profile_deviation_m(position.altitude_m, to.altitude.as_ref()),
        distance_to_waypoint_m,
        to_from: lateral_deviation_m.map(|_| {
            super::navigation_display::to_from(
                position,
                &to.position,
                course_rad,
                distance_to_waypoint_m,
            )
        }),
        leg_index: u32::try_from(leg.index).unwrap_or(u32::MAX),
        waypoint_count: u32::try_from(waypoint_count).unwrap_or(u32::MAX),
        integrity,
        holding,
        position: None,
        position_integrity: None,
        phase,
        rnp_m,
    }
}

fn leg_lateral_geometry(
    position: &GeodeticPosition,
    leg: &Leg<'_>,
    hold_status: Option<HoldStatus>,
) -> (Option<String>, f64, Option<f64>, bool) {
    let to = leg.to;
    let in_hold_circuit = hold_status.is_some_and(|status| status.stage != HoldStage::ToFix);
    match (in_hold_circuit, leg.path.hold()) {
        (true, Some(hold)) => (
            None,
            hold.inbound_course_rad,
            Some(cross_track_from_course_m(
                position,
                &to.position,
                hold.inbound_course_rad,
                GEODESY,
            )),
            true,
        ),
        _ => match leg.lateral_reference() {
            LateralReference::Track { from } => (
                leg.from.map(|fix| fix.ident.as_str().to_owned()),
                initial_bearing_rad(&from, &to.position),
                cross_track_m(position, &from, &to.position, GEODESY).ok(),
                false,
            ),
            LateralReference::Course { course_rad } => (
                leg.from.map(|fix| fix.ident.as_str().to_owned()),
                course_rad,
                Some(cross_track_from_course_m(
                    position,
                    &to.position,
                    course_rad,
                    GEODESY,
                )),
                false,
            ),
            LateralReference::DisplacedCourse { anchor, course_rad } => (
                None,
                course_rad,
                Some(cross_track_from_course_m(
                    position, &anchor, course_rad, GEODESY,
                )),
                false,
            ),
            _ => (
                None,
                initial_bearing_rad(position, &to.position),
                None,
                false,
            ),
        },
    }
}

/// Signed deviation from a waypoint's altitude constraint, positive above
/// the profile — the sign convention guidance itself derives.
fn profile_deviation_m(altitude_m: f64, constraint: Option<&AltitudeConstraint>) -> Option<f64> {
    match constraint? {
        AltitudeConstraint::At(target_m) => Some(altitude_m - target_m),
        AltitudeConstraint::AtOrAbove(floor_m) => Some((altitude_m - floor_m).min(0.0)),
        AltitudeConstraint::AtOrBelow(ceiling_m) => Some((altitude_m - ceiling_m).max(0.0)),
        AltitudeConstraint::Window { lower_m, upper_m } => {
            if altitude_m < *lower_m {
                Some(altitude_m - lower_m)
            } else if altitude_m > *upper_m {
                Some(altitude_m - upper_m)
            } else {
                Some(0.0)
            }
        }
        _ => None,
    }
}
