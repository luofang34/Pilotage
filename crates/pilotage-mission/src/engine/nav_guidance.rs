//! Display-facing navigation guidance: the plan-relative geometry the
//! executor is flying, in canonical units.
//!
//! This is display context (ADR-0031, ADR-0024): never an input to
//! control validation and never a stand-in for a missing estimate.
//! Absence is meaningful — an executor flying nothing has no guidance,
//! and consumers remove the deviation display rather than centering it.
//! Scaling deviations to instrument dots is display policy and happens
//! past this boundary (ADR-0017).

use navigate_contract::{AltitudeConstraint, GeodeticPosition, SolutionQuality};
use navigate_fpl::Leg;
use navigate_geodesy::{cross_track_m, distance_m, initial_bearing_rad};

use super::{MissionEngine, MissionState};

#[cfg(test)]
mod tests;

/// Quality of the navigation solution the guidance geometry rests on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NavQuality {
    /// Within the filter's accuracy bounds and admission health.
    Good,
    /// Usable with caution: bounds exceeded or sources degraded.
    Degraded,
    /// Not usable for any navigation decision; consumers remove the
    /// guidance display rather than annotating it.
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
    /// Identifier of the leg's origin fix; `None` on a direct-to leg,
    /// which is flown from the present position.
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
    /// Quality of the solution this geometry rests on.
    pub quality: NavQuality,
}

impl MissionEngine {
    /// The guidance the executor is flying, or `None` when it is flying
    /// nothing: before a first solution, while arming, and once the plan
    /// is complete. The geometry comes from the solution and active leg
    /// the latest [`MissionEngine::tick`] left behind, so this reads
    /// state and never re-runs fusion or sequencing.
    ///
    /// A tick whose filter published nothing clears the solution, so
    /// guidance disappears with it instead of aging silently on a
    /// display.
    #[must_use]
    pub fn nav_guidance(&self) -> Option<NavGuidance> {
        let solution = self.last_solution.as_ref()?;
        match self.state {
            MissionState::AwaitSolution | MissionState::Arming | MissionState::Complete => None,
            MissionState::Climb | MissionState::Enroute => {
                let leg = self.execution.active_leg()?;
                Some(leg_guidance(
                    &solution.position,
                    &leg,
                    self.execution.plan().waypoints.len(),
                    solution.integrity.quality.into(),
                ))
            }
        }
    }
}

/// The plan-relative geometry of one active leg from `position`.
///
/// The track runs from the leg's origin fix to its destination; a
/// direct-to leg has no origin fix, so its course is the live bearing to
/// the waypoint and no cross-track deviation exists to report.
fn leg_guidance(
    position: &GeodeticPosition,
    leg: &Leg<'_>,
    waypoint_count: usize,
    quality: NavQuality,
) -> NavGuidance {
    let to = leg.to;
    let course_rad = leg.from.map_or_else(
        || initial_bearing_rad(position, &to.position),
        |from| initial_bearing_rad(&from.position, &to.position),
    );
    NavGuidance {
        to_ident: to.ident.clone(),
        from_ident: leg.from.map(|from| from.ident.clone()),
        course_rad,
        lateral_deviation_m: leg
            .from
            .and_then(|from| cross_track_m(position, &from.position, &to.position).ok()),
        vertical_deviation_m: profile_deviation_m(position.altitude_m, to.altitude.as_ref()),
        distance_to_waypoint_m: distance_m(position, &to.position),
        leg_index: u32::try_from(leg.index).unwrap_or(u32::MAX),
        waypoint_count: u32::try_from(waypoint_count).unwrap_or(u32::MAX),
        quality,
    }
}

/// Signed deviation from a waypoint's altitude constraint, positive above
/// the profile — the sign convention guidance itself derives.
///
/// An `At` constraint reports the signed deviation both ways; the
/// one-sided forms report only their violation direction and zero while
/// satisfied. A waypoint with no constraint — or one wearing a constraint
/// this vocabulary cannot read — has no profile to deviate from and
/// reports nothing rather than an invented zero.
fn profile_deviation_m(altitude_m: f64, constraint: Option<&AltitudeConstraint>) -> Option<f64> {
    match constraint? {
        AltitudeConstraint::At(target_m) => Some(altitude_m - target_m),
        AltitudeConstraint::AtOrAbove(floor_m) => Some((altitude_m - floor_m).min(0.0)),
        AltitudeConstraint::AtOrBelow(ceiling_m) => Some((altitude_m - ceiling_m).max(0.0)),
        _ => None,
    }
}
