#![allow(clippy::expect_used)]
use super::*;
use crate::{NavigationPoint, RoutePlan, RouteWaypoint, evaluate_route};

fn profile() -> VehicleProfile {
    VehicleProfile {
        id: "test".into(),
        revision: 1,
        name: "Test profile".into(),
        registration: None,
        kind: "aircraft".into(),
        cruise_speed_knots: 120.0,
        speed_reference: SpeedReference::TrueAirspeed,
        source: Some("Test manual".into()),
    }
}

#[test]
fn profile_speed_drives_route_time_and_an_override_is_explicit() {
    let mut route = RoutePlan {
        waypoints: vec![],
        groundspeed_knots: None,
        departure_utc: None,
        vehicle_profile: Some(profile()),
    };
    for (id, lon) in [("a", 0.0), ("b", 1.0)] {
        route.waypoints.push(RouteWaypoint {
            id: id.into(),
            source: None,
            altitude_msl_ft: None,
            point: NavigationPoint {
                key: id.into(),
                identifier: id.into(),
                kind: "waypoint".into(),
                name: String::new(),
                region: String::new(),
                latitude_deg: 0.0,
                longitude_deg: lon,
            },
        });
    }
    let baseline = evaluate_route(&route)
        .expect("route")
        .duration_seconds
        .expect("time");
    assert!((baseline - 1801.2).abs() < 1.0);
    route.groundspeed_knots = Some(60.0);
    let slower = evaluate_route(&route)
        .expect("override")
        .duration_seconds
        .expect("time");
    assert!((slower - baseline * 2.0).abs() < 0.001);
}

#[test]
fn profile_import_round_trips_and_rejects_duplicate_or_invalid_data() {
    let mut document = VehicleProfileDocument {
        schema_version: 1,
        profiles: vec![profile()],
    };
    document.validate().expect("valid");
    let json = serde_json::to_string(&document).expect("encode");
    assert_eq!(
        serde_json::from_str::<VehicleProfileDocument>(&json).expect("decode"),
        document
    );
    document.profiles.push(profile());
    assert!(document.validate().is_err());
    document.profiles.pop();
    document.profiles[0].cruise_speed_knots = f64::NAN;
    assert!(document.validate().is_err());
    assert!(
        serde_json::from_str::<VehicleProfileDocument>(
            &json.replace("cruise_speed_knots", "cruise_speed_kmh")
        )
        .is_err()
    );
}
