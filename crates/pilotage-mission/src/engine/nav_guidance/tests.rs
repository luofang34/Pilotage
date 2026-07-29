#![allow(clippy::expect_used, clippy::panic)]

use navigate_contract::{AltitudeConstraint, GeodeticPosition, SolutionQuality, Waypoint};
use navigate_fpl::Leg;

use super::{NavQuality, leg_guidance, profile_deviation_m};

/// One degree of arc on the mean-radius sphere, meters: the scale every
/// hand-computed expectation below is derived from.
const METERS_PER_DEGREE: f64 = 111_195.0;

fn position(lat_deg: f64, lon_deg: f64, altitude_m: f64) -> GeodeticPosition {
    GeodeticPosition::new(lat_deg.to_radians(), lon_deg.to_radians(), altitude_m)
}

/// The eastbound equatorial leg every deviation expectation is measured
/// against: `EASTA` at 0°E, `EASTB` at 1°E, both crossing at 1000 m.
fn eastbound_leg() -> (Waypoint, Waypoint) {
    (
        Waypoint::new("EASTA".to_owned(), position(0.0, 0.0, 1000.0)),
        Waypoint::new("EASTB".to_owned(), position(0.0, 1.0, 1000.0))
            .with_altitude(AltitudeConstraint::At(1000.0)),
    )
}

#[test]
fn eastbound_leg_pins_course_deviations_and_distance() {
    let (from, to) = eastbound_leg();
    let leg = Leg {
        from: Some(&from),
        to: &to,
        index: 1,
    };
    // 0.01° south of the track at its midpoint, 50 m above the profile.
    let ownship = position(-0.01, 0.5, 1050.0);

    let guidance = leg_guidance(&ownship, &leg, 3, NavQuality::Good);

    assert_eq!(guidance.to_ident, "EASTB");
    assert_eq!(guidance.from_ident.as_deref(), Some("EASTA"));
    assert_eq!(guidance.leg_index, 1);
    assert_eq!(guidance.waypoint_count, 3);
    assert_eq!(guidance.quality, NavQuality::Good);

    // An equatorial eastbound great circle courses due east exactly.
    let course_error = guidance.course_rad - std::f64::consts::FRAC_PI_2;
    assert!(course_error.abs() < 1e-12, "course {}", guidance.course_rad);

    // Right of an eastbound track is south, and 0.01° of arc is 1112 m.
    let lateral = guidance
        .lateral_deviation_m
        .expect("a leg with an origin fix has cross-track geometry");
    let expected_lateral = 0.01 * METERS_PER_DEGREE;
    assert!(
        (lateral - expected_lateral).abs() < 1.0,
        "right of track must be positive {expected_lateral} m, got {lateral}"
    );

    // 0.5° of longitude ahead and 0.01° of latitude aside: the
    // great-circle leg is hypot(0.5, 0.01)° long.
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
    let (from, to) = eastbound_leg();
    let leg = Leg {
        from: Some(&from),
        to: &to,
        index: 1,
    };
    let guidance = leg_guidance(&position(0.01, 0.5, 1000.0), &leg, 3, NavQuality::Degraded);

    let lateral = guidance
        .lateral_deviation_m
        .expect("a leg with an origin fix has cross-track geometry");
    assert!(
        (lateral + 0.01 * METERS_PER_DEGREE).abs() < 1.0,
        "left of track must be negative, got {lateral}"
    );
    assert_eq!(guidance.quality, NavQuality::Degraded);
    assert_eq!(guidance.vertical_deviation_m, Some(0.0));
}

#[test]
fn direct_to_courses_at_the_waypoint_and_reports_no_cross_track() {
    let (_, to) = eastbound_leg();
    let leg = Leg {
        from: None,
        to: &to,
        index: 0,
    };
    // Due south-west of the waypoint: the live bearing to it is the
    // course, and no track exists to deviate from.
    let guidance = leg_guidance(&position(-0.01, 0.5, 1000.0), &leg, 3, NavQuality::Good);

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
fn an_unconstrained_waypoint_reports_no_vertical_deviation() {
    let (from, _) = eastbound_leg();
    let unconstrained = Waypoint::new("FREEE".to_owned(), position(0.0, 1.0, 1000.0));
    let leg = Leg {
        from: Some(&from),
        to: &unconstrained,
        index: 1,
    };
    let guidance = leg_guidance(&position(0.0, 0.5, 2000.0), &leg, 2, NavQuality::Good);
    assert_eq!(guidance.vertical_deviation_m, None);
}

#[test]
fn one_sided_constraints_report_only_their_violation_direction() {
    let floor = AltitudeConstraint::AtOrAbove(1000.0);
    assert_eq!(profile_deviation_m(1500.0, Some(&floor)), Some(0.0));
    assert_eq!(profile_deviation_m(900.0, Some(&floor)), Some(-100.0));

    let ceiling = AltitudeConstraint::AtOrBelow(1000.0);
    assert_eq!(profile_deviation_m(500.0, Some(&ceiling)), Some(0.0));
    assert_eq!(profile_deviation_m(1100.0, Some(&ceiling)), Some(100.0));

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
