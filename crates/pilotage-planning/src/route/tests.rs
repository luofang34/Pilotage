#![allow(clippy::expect_used, clippy::panic)]
use super::*;

fn point(id: &str, lon: f64) -> RouteWaypoint {
    RouteWaypoint {
        id: id.into(),
        point: NavigationPoint {
            key: id.into(),
            identifier: id.into(),
            kind: "waypoint".into(),
            name: String::new(),
            region: String::new(),
            latitude_deg: 0.0,
            longitude_deg: lon,
        },
        source: None,
        altitude_msl_ft: None,
    }
}

#[test]
fn route_estimates_change_with_order_speed_and_departure() {
    let mut route = RoutePlan {
        waypoints: vec![point("a", 179.0), point("b", -179.0)],
        groundspeed_knots: Some(120.0),
        departure_utc: Some(1_000),
        vehicle_profile: None,
    };
    let summary = evaluate_route(&route).expect("estimate");
    assert!((summary.distance_nm - 120.08).abs() < 0.1);
    assert_eq!(summary.legs[1].track_true_deg, Some(90.0));
    assert!((summary.duration_seconds.expect("duration") - 3602.4).abs() < 1.0);
    assert_eq!(summary.legs[1].remaining_nm, 0.0);
    assert!(summary.legs[1].arrival_utc.expect("arrival") > 4600.0);
    route.waypoints.reverse();
    assert_eq!(
        evaluate_route(&route).expect("reverse").legs[1].track_true_deg,
        Some(270.0)
    );
    route.groundspeed_knots = None;
    let unknown = evaluate_route(&route).expect("no speed");
    assert_eq!(unknown.duration_seconds, None);
    assert_eq!(unknown.legs[1].arrival_utc, None);
}

#[test]
fn invalid_points_and_duplicate_occurrences_fail() {
    let mut route = RoutePlan {
        waypoints: vec![point("a", 0.0), point("a", 1.0)],
        groundspeed_knots: None,
        departure_utc: None,
        vehicle_profile: None,
    };
    assert!(evaluate_route(&route).is_err());
    route.waypoints.pop();
    route.waypoints[0].point.longitude_deg = f64::INFINITY;
    assert!(evaluate_route(&route).is_err());
}

#[test]
fn time_overflow_is_not_serialized_as_an_unknown_estimate() {
    let mut route = RoutePlan {
        waypoints: vec![point("a", 0.0), point("b", 1.0)],
        groundspeed_knots: Some(f64::MIN_POSITIVE),
        departure_utc: Some(1),
        vehicle_profile: None,
    };
    assert!(evaluate_route(&route).is_err());
    route.groundspeed_knots = Some(1e-100);
    assert!(evaluate_route(&route).is_err());
    route.groundspeed_knots = Some(100.0);
    route.departure_utc = Some(LAST_PLANNING_UTC);
    assert!(evaluate_route(&route).is_err());
    route.departure_utc = Some(i64::MAX);
    assert!(evaluate_route(&route).is_err());
}

#[test]
fn validity_is_checked_at_each_arrival_and_expiry_is_exclusive() {
    let source = NavigationSource {
        release_id: "release".into(),
        authority: "provider".into(),
        edition: "cycle".into(),
        source_digest: "a".repeat(64),
        effective_at: 10,
        expires_at: 20,
    };
    let mut route = RoutePlan {
        waypoints: vec![point("a", 0.0)],
        groundspeed_knots: Some(100.0),
        departure_utc: Some(20),
        vehicle_profile: None,
    };
    route.waypoints[0].source = Some(source);
    assert!(
        evaluate_route(&route)
            .expect("expiry")
            .issues
            .iter()
            .any(|s| s.contains("outside"))
    );
    route.departure_utc = Some(10);
    assert!(
        !evaluate_route(&route)
            .expect("effective")
            .issues
            .iter()
            .any(|s| s.contains("outside"))
    );
}
