//! Time-on-target estimates and departure-window intersections.

use super::CoordinatedPlan;
use crate::{PlanningError, RouteSummary, evaluate_route};
use serde::Serialize;
use std::collections::BTreeMap;

/// Calculated timing for one vehicle at one target.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TimingAssessment {
    /// Timing condition identity.
    pub target_id: String,
    /// Vehicle assignment identity.
    pub assignment_id: String,
    /// Target route occurrence identity.
    pub waypoint_id: String,
    /// Target plus member offset, in UTC Unix seconds.
    pub required_utc: f64,
    /// Estimated arrival, when departure and ground speed are supplied.
    pub estimated_utc: Option<f64>,
    /// Estimated arrival minus required arrival, in seconds.
    pub deviation_seconds: Option<f64>,
    /// Earliest departure that meets this target at planned ground speed.
    pub earliest_departure_utc: Option<f64>,
    /// Latest departure that meets this target at planned ground speed.
    pub latest_departure_utc: Option<f64>,
    /// `on_time`, `early`, `late`, or `unknown`.
    pub status: String,
}

/// Estimates for a complete coordinated draft.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CoordinationSummary {
    /// Route estimates keyed by assignment identity.
    pub routes: BTreeMap<String, RouteSummary>,
    /// Individual and swarm timing results.
    pub timings: Vec<TimingAssessment>,
    /// Conflicting timing windows or duplicate vehicle assignments.
    pub issues: Vec<String>,
}

/// Checks time-on-target constraints at each member's selected route occurrence.
pub fn assess_coordination(plan: &CoordinatedPlan) -> Result<CoordinationSummary, PlanningError> {
    plan.validate()?;
    let mut routes = BTreeMap::new();
    for assignment in &plan.assignments {
        routes.insert(assignment.id.clone(), evaluate_route(&assignment.route)?);
    }
    let mut timings = Vec::new();
    let mut windows: BTreeMap<&str, (f64, f64)> = BTreeMap::new();
    for target in &plan.timing_targets {
        for member in &target.members {
            let row = routes
                .get(&member.assignment_id)
                .and_then(|r| r.legs.iter().find(|l| l.waypoint_id == member.waypoint_id));
            let elapsed = row.and_then(|r| r.elapsed_seconds);
            let estimated = row.and_then(|r| r.arrival_utc);
            let required = target.utc as f64 + f64::from(member.offset_seconds);
            let tolerance = f64::from(target.tolerance_seconds);
            let low = elapsed.map(|t| required - tolerance - t);
            let high = elapsed.map(|t| required + tolerance - t);
            if let (Some(low), Some(high)) = (low, high) {
                let window = windows.entry(&member.assignment_id).or_insert((low, high));
                window.0 = window.0.max(low);
                window.1 = window.1.min(high);
            }
            let deviation = estimated.map(|t| t - required);
            timings.push(TimingAssessment {
                target_id: target.id.clone(),
                assignment_id: member.assignment_id.clone(),
                waypoint_id: member.waypoint_id.clone(),
                required_utc: required,
                estimated_utc: estimated,
                deviation_seconds: deviation,
                earliest_departure_utc: low,
                latest_departure_utc: high,
                status: status(deviation, tolerance).into(),
            });
        }
    }
    let mut issues: Vec<_> = windows.into_iter().filter(|(_, (low, high))| low > high)
        .map(|(id, _)| format!("{id}: target windows require different departure times at the entered ground speed.")).collect();
    let mut vehicles = BTreeMap::new();
    for assignment in &plan.assignments {
        if let Some(vehicle) = &assignment.vehicle_reference
            && let Some(other) = vehicles.insert(vehicle, &assignment.id)
        {
            issues.push(format!(
                "{vehicle}: assigned to both {other} and {}. Check the schedule.",
                assignment.id
            ));
        }
    }
    Ok(CoordinationSummary {
        routes,
        timings,
        issues,
    })
}

fn status(deviation: Option<f64>, tolerance: f64) -> &'static str {
    match deviation {
        None => "unknown",
        Some(d) if d < -tolerance => "early",
        Some(d) if d > tolerance => "late",
        Some(_) => "on_time",
    }
}
