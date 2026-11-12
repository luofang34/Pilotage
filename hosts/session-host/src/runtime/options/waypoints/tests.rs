use pilotage_mission::{
    LegRole, PlannedAltitude, PlannedCourseReference, PlannedHold, PlannedHoldLength, PlannedPath,
    WaypointTurn, WaypointTurnDirection,
};

use super::parse;

#[test]
fn a_waypoint_file_becomes_a_waypoint_plan() {
    let plan = parse(
        r#"{
            "capture_radius_m": 5.0,
            "arrival_radius_m": 0.5,
            "waypoints": [
                { "lat_deg": 47.0006, "lon_deg": 8.0, "altitude": { "at": 512.0 } },
                { "lat_deg": 47.0006, "lon_deg": 8.0008, "altitude": { "at": 518.0 },
                  "turn": "fly-over", "name": "CORNER" }
            ]
        }"#,
    )
    .expect("parses");
    assert_eq!(plan.capture_radius_m, Some(5.0));
    assert_eq!(plan.arrival_radius_m, 0.5);
    let derived = parse(r#"{ "waypoints": [] }"#).expect("parses");
    assert_eq!(
        derived.capture_radius_m, None,
        "the radius follows the cruise speed"
    );
    assert_eq!(plan.waypoints.len(), 2);
    assert_eq!(plan.waypoints[0].turn, WaypointTurn::FlyBy);
    assert_eq!(plan.waypoints[0].name, None);
    assert_eq!(plan.waypoints[0].path, None);
    assert_eq!(plan.waypoints[0].max_speed_mps, None);
    assert_eq!(plan.waypoints[0].gradient, None);
    assert_eq!(plan.waypoints[1].turn, WaypointTurn::FlyOver);
    assert_eq!(plan.waypoints[1].name.as_deref(), Some("CORNER"));
    assert_eq!(plan.waypoints[1].altitude, PlannedAltitude::At(518.0));
}

#[test]
fn constraints_holds_roles_and_leg_paths_reach_the_plan() {
    let plan = parse(
        r#"{
            "waypoints": [
                { "lat_deg": 47.0, "lon_deg": 8.0, "altitude": { "at-or-above": 500.0 },
                  "path": "initial-fix", "role": "arrival" },
                { "lat_deg": 47.0, "lon_deg": 8.01,
                  "altitude": { "window": { "lower_m": 500.0, "upper_m": 540.0 } },
                  "path": { "course-to-fix": { "course_deg": 90.0, "reference": "magnetic" } },
                  "turn_direction": "left", "max_speed_mps": 25.0, "gradient": -0.05,
                  "role": "approach", "rnp_nm": 0.15 },
                { "lat_deg": 47.01, "lon_deg": 8.01, "altitude": { "at-or-above": 700.0 },
                  "path": { "course-to-altitude": { "course_deg": 0.0, "reference": "magnetic" } },
                  "role": "missed-approach" },
                { "lat_deg": 47.02, "lon_deg": 8.01, "altitude": { "at": 900.0 },
                  "path": { "hold-manual": {
                      "inbound_course_deg": 180.0,
                      "reference": "true",
                      "turn": "right",
                      "length": { "minutes": 1.0 }
                  } },
                  "role": "missed-approach" }
            ]
        }"#,
    )
    .expect("parses");
    let [ifix, faf, ca, mahf] = plan.waypoints.as_slice() else {
        panic!("expected four waypoints");
    };
    assert_eq!(ifix.path, Some(PlannedPath::InitialFix));
    assert_eq!(ifix.role, Some(LegRole::Arrival));
    assert_eq!(
        faf.path,
        Some(PlannedPath::CourseToFix {
            course_rad: 90.0_f64.to_radians(),
            reference: PlannedCourseReference::Magnetic,
        })
    );
    assert_eq!(faf.turn_direction, Some(WaypointTurnDirection::Left));
    assert_eq!(faf.role, Some(LegRole::Approach));
    assert_eq!(faf.rnp_nm, Some(0.15));
    assert_eq!(
        ca.path,
        Some(PlannedPath::CourseToAltitude {
            course_rad: 0.0,
            reference: PlannedCourseReference::Magnetic,
        })
    );
    assert_eq!(
        mahf.path,
        Some(PlannedPath::HoldManual(PlannedHold {
            inbound_course_rad: 180.0_f64.to_radians(),
            reference: PlannedCourseReference::True,
            turn: WaypointTurnDirection::Right,
            length: PlannedHoldLength::Minutes(1.0),
        }))
    );
}

#[test]
fn a_file_that_is_not_a_waypoint_list_is_refused() {
    for text in [
        "not json",
        r#"{ "waypoints": [ { "lat_deg": 47.0, "lon_deg": 8.0 } ] }"#,
        r#"{ "waypoints": [ { "lat_deg": 47.0, "lon_deg": 8.0, "alt_m": 1.0 } ] }"#,
        r#"{ "waypoints": [], "extra": 1 }"#,
        r#"{ "waypoints": [ { "lat_deg": 47.0, "lon_deg": 8.0, "altitude": { "at": 1.0 }, "turn": "loop" } ] }"#,
        r#"{ "waypoints": [ { "lat_deg": 47.0, "lon_deg": 8.0, "altitude": { "near": 1.0 } } ] }"#,
        r#"{ "waypoints": [ { "lat_deg": 47.0, "lon_deg": 8.0, "altitude": { "window": { "lower_m": 1.0 } } } ] }"#,
        r#"{ "waypoints": [ { "lat_deg": 47.0, "lon_deg": 8.0, "altitude": { "at": 1.0 }, "path": "track-to-fix" } ] }"#,
        r#"{ "waypoints": [ { "lat_deg": 47.0, "lon_deg": 8.0, "altitude": { "at": 1.0 },
            "path": { "course-to-fix": { "course_deg": 90.0 } } } ] }"#,
        r#"{ "capture_radius_m": 0.0, "waypoints": [] }"#,
        r#"{ "arrival_radius_m": -1.0, "waypoints": [] }"#,
    ] {
        assert!(parse(text).is_err(), "{text}");
    }
}
