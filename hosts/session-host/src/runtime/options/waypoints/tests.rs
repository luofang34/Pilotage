use pilotage_mission::WaypointTurn;

use super::parse;

#[test]
fn a_waypoint_file_becomes_a_waypoint_plan() {
    let plan = parse(
        r#"{
            "capture_radius_m": 5.0,
            "waypoints": [
                { "lat_deg": 47.0006, "lon_deg": 8.0, "alt_m": 512.0 },
                { "lat_deg": 47.0006, "lon_deg": 8.0008, "alt_m": 518.0,
                  "turn": "fly-over", "name": "CORNER" }
            ]
        }"#,
    )
    .expect("parses");
    assert_eq!(plan.capture_radius_m, 5.0);
    assert_eq!(plan.waypoints.len(), 2);
    assert_eq!(plan.waypoints[0].turn, WaypointTurn::FlyBy);
    assert_eq!(plan.waypoints[0].name, None);
    assert_eq!(plan.waypoints[1].turn, WaypointTurn::FlyOver);
    assert_eq!(plan.waypoints[1].name.as_deref(), Some("CORNER"));
    assert_eq!(plan.waypoints[1].altitude_m, 518.0);
}

#[test]
fn a_file_that_is_not_a_waypoint_list_is_refused() {
    for text in [
        "not json",
        r#"{ "waypoints": [ { "lat_deg": 47.0, "lon_deg": 8.0 } ] }"#,
        r#"{ "waypoints": [], "extra": 1 }"#,
        r#"{ "waypoints": [ { "lat_deg": 47.0, "lon_deg": 8.0, "alt_m": 1.0, "turn": "loop" } ] }"#,
        r#"{ "capture_radius_m": 0.0, "waypoints": [] }"#,
    ] {
        assert!(parse(text).is_err(), "{text}");
    }
}
