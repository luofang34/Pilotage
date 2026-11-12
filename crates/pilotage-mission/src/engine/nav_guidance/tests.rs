#![allow(clippy::expect_used, clippy::panic)]

use core::f64::consts::FRAC_PI_2;

use navigate_contract::{
    AltitudeConstraint, CourseReference, Ellipsoid, FlightPlan, GeodeticPosition, HoldGeometry,
    HoldLegLength, Ident, LegPath, PlanRole, SolutionQuality, TurnDirection, Waypoint,
};
use navigate_route::{ExecutionConfig, HoldEntry, HoldStage, HoldStatus, PlanExecution};

use super::{NavQuality, SteeringIntegrity, leg_guidance, profile_deviation_m};
use crate::FlightPhase;

/// A solution within its limits at the navigator's `quality`.
const fn within(quality: NavQuality) -> SteeringIntegrity {
    SteeringIntegrity::WithinLimits {
        quality,
        horizontal_1sigma_m: 5.0,
    }
}

fn ident(name: &str) -> Ident {
    Ident::try_new(name).expect("a short name")
}

const METERS_PER_DEGREE: f64 = 111_195.0;

fn position(lat_deg: f64, lon_deg: f64, altitude_m: f64) -> GeodeticPosition {
    GeodeticPosition::new(lat_deg.to_radians(), lon_deg.to_radians(), altitude_m)
}

fn eastbound_waypoints() -> [Waypoint; 2] {
    [
        Waypoint::new(ident("EASTA"), position(0.0, 0.0, 1000.0)),
        Waypoint::new(ident("EASTB"), position(0.0, 1.0, 1000.0))
            .with_altitude(AltitudeConstraint::At(1000.0)),
    ]
}

fn execution(waypoints: &[Waypoint], advance_from: Option<GeodeticPosition>) -> PlanExecution<'_> {
    let plan = FlightPlan::new(
        ident("TEST"),
        PlanRole::Mission,
        Ellipsoid::WGS84,
        waypoints,
    );
    let mut exec = PlanExecution::new(plan, ExecutionConfig::default()).expect("exec");
    if let Some(pos) = advance_from {
        let _ = exec.advance(&pos, 50.0);
    }
    exec
}

#[test]
fn eastbound_leg_pins_course_deviations_and_distance() {
    let wps = eastbound_waypoints();
    let exec = execution(&wps, Some(position(0.0, 0.0, 1000.0)));
    let leg = exec.active_leg().expect("active leg");
    let ownship = position(-0.01, 0.5, 1050.0);

    let guidance = leg_guidance(
        &ownship,
        &leg,
        None,
        3,
        within(NavQuality::Good),
        FlightPhase::Enroute,
        3_704.0,
    );

    assert_eq!(guidance.to_ident, "EASTB");
    assert_eq!(guidance.from_ident.as_deref(), Some("EASTA"));
    assert_eq!(guidance.leg_index, 1);
    assert_eq!(guidance.waypoint_count, 3);
    assert_eq!(guidance.integrity, within(NavQuality::Good));
    assert_eq!(guidance.phase, FlightPhase::Enroute);
    assert!((guidance.rnp_m - 3_704.0).abs() < 1e-9);

    let course_error = guidance.course_rad - FRAC_PI_2;
    assert!(course_error.abs() < 1e-12, "course {}", guidance.course_rad);

    let lateral = guidance
        .lateral_deviation_m
        .expect("a leg with an origin fix has cross-track geometry");
    let expected_lateral = 0.01 * METERS_PER_DEGREE;
    assert!(
        (lateral - expected_lateral).abs() < 1.0,
        "right of track must be positive {expected_lateral} m, got {lateral}"
    );

    let expected_distance = 0.5_f64.hypot(0.01) * METERS_PER_DEGREE;
    assert!(
        (guidance.distance_to_waypoint_m - expected_distance).abs() < 5.0,
        "distance {expected_distance} m, got {}",
        guidance.distance_to_waypoint_m
    );

    assert_eq!(guidance.vertical_deviation_m, Some(50.0));
}

#[test]
fn left_of_track_is_negative() {
    let wps = eastbound_waypoints();
    let exec = execution(&wps, Some(position(0.0, 0.0, 1000.0)));
    let leg = exec.active_leg().expect("active leg");
    let guidance = leg_guidance(
        &position(0.01, 0.5, 1000.0),
        &leg,
        None,
        3,
        within(NavQuality::Degraded),
        FlightPhase::Terminal,
        1_852.0,
    );

    let lateral = guidance
        .lateral_deviation_m
        .expect("a leg with an origin fix has cross-track geometry");
    assert!(
        (lateral + 0.01 * METERS_PER_DEGREE).abs() < 1.0,
        "left of track must be negative, got {lateral}"
    );
    assert_eq!(guidance.integrity, within(NavQuality::Degraded));
    assert_eq!(guidance.vertical_deviation_m, Some(0.0));
    assert_eq!(guidance.phase, FlightPhase::Terminal);
    assert!((guidance.rnp_m - 1_852.0).abs() < 1e-9);
}

#[test]
fn direct_to_courses_at_the_waypoint_and_reports_no_cross_track() {
    let wps = [Waypoint::new(ident("EASTB"), position(0.0, 1.0, 1000.0))
        .with_path(LegPath::DirectToFix)
        .with_altitude(AltitudeConstraint::At(1000.0))];
    let exec = execution(&wps, None);
    let leg = exec.active_leg().expect("active leg");
    let guidance = leg_guidance(
        &position(-0.01, 0.5, 1000.0),
        &leg,
        None,
        3,
        within(NavQuality::Good),
        FlightPhase::Enroute,
        3_704.0,
    );

    assert_eq!(guidance.from_ident, None);
    assert_eq!(guidance.lateral_deviation_m, None);
    assert_eq!(guidance.leg_index, 0);
    let bearing_deg = guidance.course_rad.to_degrees();
    assert!(
        (0.0..90.0).contains(&bearing_deg),
        "a waypoint north-east of ownship bears between north and east, got {bearing_deg}"
    );
}

#[test]
fn course_to_fix_and_active_hold_report_published_course_and_cross_track() {
    let wps = [
        Waypoint::new(ident("IFIX"), position(0.0, 0.0, 1000.0)),
        Waypoint::new(ident("FAF"), position(0.0, 0.2, 600.0))
            .with_path(LegPath::CourseToFix {
                course_rad: FRAC_PI_2,
                reference: CourseReference::True,
            })
            .with_altitude(AltitudeConstraint::At(600.0)),
        Waypoint::new(ident("MAHF"), position(0.0, 0.4, 900.0)).with_path(LegPath::HoldManual {
            hold: HoldGeometry::new(
                FRAC_PI_2,
                CourseReference::True,
                TurnDirection::Right,
                HoldLegLength::Time { seconds: 60.0 },
            ),
        }),
    ];
    let mut exec = execution(&wps, Some(position(0.0, 0.0, 1000.0)));
    let cf_leg = exec.active_leg().expect("cf leg");
    let cf_guidance = leg_guidance(
        &position(-0.005, 0.1, 600.0),
        &cf_leg,
        None,
        3,
        within(NavQuality::Good),
        FlightPhase::Approach,
        555.6,
    );
    assert!((cf_guidance.course_rad - FRAC_PI_2).abs() < 1e-12);
    assert!(cf_guidance.lateral_deviation_m.expect("cf xtk") > 500.0);
    assert!(!cf_guidance.holding);

    let _ = exec.advance(&position(0.0, 0.2, 600.0), 50.0);
    let hold_leg = exec.active_leg().expect("hold leg");
    let hold_guidance = leg_guidance(
        &position(-0.005, 0.35, 900.0),
        &hold_leg,
        Some(HoldStatus {
            entry: Some(HoldEntry::Direct),
            stage: HoldStage::Outbound,
            circuits_completed: 0,
            exit_armed: false,
        }),
        3,
        within(NavQuality::Good),
        FlightPhase::Missed,
        1_852.0,
    );
    assert!(hold_guidance.holding);
    assert!((hold_guidance.course_rad - FRAC_PI_2).abs() < 1e-12);
    assert!(hold_guidance.lateral_deviation_m.expect("hold xtk") > 500.0);
}

#[test]
fn an_unconstrained_waypoint_reports_no_vertical_deviation() {
    let wps = [
        Waypoint::new(ident("EASTA"), position(0.0, 0.0, 1000.0)),
        Waypoint::new(ident("FREEE"), position(0.0, 1.0, 1000.0)),
    ];
    let exec = execution(&wps, Some(position(0.0, 0.0, 1000.0)));
    let leg = exec.active_leg().expect("active leg");
    let guidance = leg_guidance(
        &position(0.0, 0.5, 2000.0),
        &leg,
        None,
        2,
        within(NavQuality::Good),
        FlightPhase::Enroute,
        3_704.0,
    );
    assert_eq!(guidance.vertical_deviation_m, None);
}

#[test]
fn one_sided_and_window_constraints_report_only_their_violation_direction() {
    let floor = AltitudeConstraint::AtOrAbove(1000.0);
    assert_eq!(profile_deviation_m(1500.0, Some(&floor)), Some(0.0));
    assert_eq!(profile_deviation_m(900.0, Some(&floor)), Some(-100.0));

    let ceiling = AltitudeConstraint::AtOrBelow(1000.0);
    assert_eq!(profile_deviation_m(500.0, Some(&ceiling)), Some(0.0));
    assert_eq!(profile_deviation_m(1100.0, Some(&ceiling)), Some(100.0));

    let window = AltitudeConstraint::Window {
        lower_m: 800.0,
        upper_m: 1200.0,
    };
    assert_eq!(profile_deviation_m(1000.0, Some(&window)), Some(0.0));
    assert_eq!(profile_deviation_m(750.0, Some(&window)), Some(-50.0));
    assert_eq!(profile_deviation_m(1275.0, Some(&window)), Some(75.0));

    assert_eq!(profile_deviation_m(1100.0, None), None);
}

#[test]
fn an_unreadable_solution_quality_is_unusable() {
    assert_eq!(NavQuality::from(SolutionQuality::Good), NavQuality::Good);
    assert_eq!(
        NavQuality::from(SolutionQuality::Degraded),
        NavQuality::Degraded
    );
    assert_eq!(
        NavQuality::from(SolutionQuality::Unusable),
        NavQuality::Unusable
    );
}
