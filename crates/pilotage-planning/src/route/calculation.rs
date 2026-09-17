//! Great-circle route estimates. Tracks use true north.

use super::{RouteLeg, RoutePlan, RouteSummary};
use crate::{NavigationPoint, PlanningError};

/// Calculates route distance, track, and time without a vehicle side effect.
pub fn evaluate_route(route: &RoutePlan) -> Result<RouteSummary, PlanningError> {
    route.validate()?;
    let mut distance_nm = 0.0;
    let mut legs = Vec::new();
    let mut issues = std::collections::BTreeSet::new();
    if route.waypoints.len() < 2 {
        issues.insert("Add at least two waypoints.".into());
    }
    if route.planning_speed_knots().is_none() {
        issues.insert("Select a vehicle profile to calculate time.".into());
    }
    for (index, waypoint) in route.waypoints.iter().enumerate() {
        let (distance, track) = if index == 0 {
            (0.0, None)
        } else {
            leg(&route.waypoints[index - 1].point, &waypoint.point)
        };
        distance_nm += distance;
        let elapsed_seconds = route
            .planning_speed_knots()
            .map(|speed| distance_nm / speed * 3600.0);
        let arrival_utc = route
            .departure_utc
            .zip(elapsed_seconds)
            .map(|(start, elapsed)| start as f64 + elapsed);
        check_time_range(elapsed_seconds, arrival_utc)?;
        if let Some(source) = &waypoint.source {
            if let Some(arrival) = arrival_utc {
                if arrival < source.effective_at as f64 || arrival >= source.expires_at as f64 {
                    issues.insert(format!(
                        "{}: {} data is outside its valid period at arrival.",
                        waypoint.point.identifier, source.edition
                    ));
                }
            } else {
                issues.insert(
                    "Set departure time and ground speed to check data validity at arrival.".into(),
                );
            }
        }
        legs.push(RouteLeg {
            waypoint_id: waypoint.id.clone(),
            distance_nm: distance,
            track_true_deg: track,
            remaining_nm: distance_nm,
            elapsed_seconds,
            arrival_utc,
        });
    }
    for leg in &mut legs {
        leg.remaining_nm = (distance_nm - leg.remaining_nm).max(0.0);
    }
    Ok(RouteSummary {
        distance_nm,
        duration_seconds: route
            .planning_speed_knots()
            .map(|speed| distance_nm / speed * 3600.0),
        legs,
        issues: issues.into_iter().collect(),
    })
}

fn check_time_range(elapsed: Option<f64>, arrival: Option<f64>) -> Result<(), PlanningError> {
    let range = 0.0..=super::LAST_PLANNING_UTC as f64;
    if elapsed
        .into_iter()
        .chain(arrival)
        .any(|value| !range.contains(&value))
    {
        return Err(crate::error::invalid(
            "route time",
            "entered speed or departure puts the estimate outside planning limits",
        ));
    }
    Ok(())
}

fn leg(from: &NavigationPoint, to: &NavigationPoint) -> (f64, Option<f64>) {
    let a = from.latitude_deg.to_radians();
    let b = to.latitude_deg.to_radians();
    let longitude = (to.longitude_deg - from.longitude_deg).to_radians();
    let haversine =
        ((b - a) / 2.0).sin().powi(2) + a.cos() * b.cos() * (longitude / 2.0).sin().powi(2);
    let distance = 2.0 * haversine.clamp(0.0, 1.0).sqrt().asin() * 6_371_008.8 / 1852.0;
    let y = longitude.sin() * b.cos();
    let x = a.cos() * b.sin() - a.sin() * b.cos() * longitude.cos();
    let track = (distance > 0.000001 && (x.abs() + y.abs()) > 1e-12)
        .then(|| y.atan2(x).to_degrees().rem_euclid(360.0));
    (distance, track)
}
