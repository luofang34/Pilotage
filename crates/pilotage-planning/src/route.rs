//! Resolved route drafts and great-circle planning estimates.

mod calculation;
#[cfg(test)]
mod tests;

use crate::{NavigationPoint, NavigationSource, PlanningError, error::invalid, navigation::text};
use serde::{Deserialize, Serialize};

pub use calculation::evaluate_route;

pub(crate) const LAST_PLANNING_UTC: i64 = 253_402_300_799;

/// A selected or manually placed waypoint in a route.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteWaypoint {
    /// Route occurrence identity. A route can visit the same source point twice.
    pub id: String,
    /// Fixed coordinates and label used by this draft.
    pub point: NavigationPoint,
    /// Source edition. Absent only for a point entered by the user.
    pub source: Option<NavigationSource>,
    /// Planned mean-sea-level altitude in feet, when supplied.
    pub altitude_msl_ft: Option<f64>,
}

/// A route for one vehicle assignment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutePlan {
    /// Ordered resolved waypoints.
    pub waypoints: Vec<RouteWaypoint>,
    /// Planned ground speed in knots. This is not a performance prediction.
    pub groundspeed_knots: Option<f64>,
    /// Planned departure time in UTC Unix seconds.
    pub departure_utc: Option<i64>,
    /// Fixed vehicle profile used by this route. Absent in an unresolved draft.
    pub vehicle_profile: Option<crate::VehicleProfile>,
}

/// Calculated values for arrival at one route point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RouteLeg {
    /// Route occurrence identity.
    pub waypoint_id: String,
    /// Distance from the preceding waypoint, in nautical miles.
    pub distance_nm: f64,
    /// Initial great-circle track in degrees true. Absent for a zero-length leg.
    pub track_true_deg: Option<f64>,
    /// Distance from this waypoint to the final waypoint, in nautical miles.
    pub remaining_nm: f64,
    /// Time from departure to this waypoint, in seconds, at planned ground speed.
    pub elapsed_seconds: Option<f64>,
    /// Estimated arrival UTC Unix seconds.
    pub arrival_utc: Option<f64>,
}

/// A route estimate and data-validity findings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RouteSummary {
    /// Total great-circle distance in nautical miles.
    pub distance_nm: f64,
    /// Estimated elapsed time at the entered ground speed, in seconds.
    pub duration_seconds: Option<f64>,
    /// One row per waypoint, in route order.
    pub legs: Vec<RouteLeg>,
    /// Conditions that require review before execution.
    pub issues: Vec<String>,
}

impl RoutePlan {
    /// Checks waypoint identities, positions, source metadata, and entered speed.
    pub fn validate(&self) -> Result<(), PlanningError> {
        if let Some(profile) = &self.vehicle_profile {
            profile.validate()?;
        }
        if self
            .departure_utc
            .is_some_and(|value| !(0..=LAST_PLANNING_UTC).contains(&value))
        {
            return Err(invalid("departure_utc", "date is outside planning limits"));
        }
        if self.waypoints.len() > 512 {
            return Err(invalid("waypoints", "more than 512 points"));
        }
        if self
            .groundspeed_knots
            .is_some_and(|v| !v.is_finite() || v <= 0.0 || v > 3000.0)
        {
            return Err(invalid(
                "groundspeed_knots",
                "expected a speed above zero and at most 3000 knots",
            ));
        }
        let mut ids = std::collections::BTreeSet::new();
        for waypoint in &self.waypoints {
            text("waypoint.id", &waypoint.id)?;
            if !ids.insert(&waypoint.id) {
                return Err(invalid(&waypoint.id, "duplicate route occurrence"));
            }
            waypoint.point.validate()?;
            if let Some(source) = &waypoint.source {
                source.validate()?;
            }
            if waypoint
                .altitude_msl_ft
                .is_some_and(|v| !v.is_finite() || !(-2000.0..=100000.0).contains(&v))
            {
                return Err(invalid(&waypoint.id, "altitude is outside planning limits"));
            }
        }
        Ok(())
    }

    /// Returns an explicit ground-speed override or the selected profile cruise speed.
    pub fn planning_speed_knots(&self) -> Option<f64> {
        self.groundspeed_knots.or_else(|| {
            self.vehicle_profile
                .as_ref()
                .map(|profile| profile.cruise_speed_knots)
        })
    }
}
