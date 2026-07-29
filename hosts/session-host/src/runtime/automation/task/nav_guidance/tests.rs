#![allow(clippy::expect_used, clippy::panic)]

use pilotage_mission::{NavGuidance, NavQuality};
use pilotage_protocol::{SessionId, wire};

use super::{NavGuidancePublisher, NavPublication};

fn guidance() -> NavGuidance {
    NavGuidance {
        to_ident: "DEMOB".to_owned(),
        from_ident: Some("DEMOA".to_owned()),
        course_rad: 1.25,
        lateral_deviation_m: Some(-12.5),
        vertical_deviation_m: Some(3.0),
        distance_to_waypoint_m: 480.0,
        leg_index: 1,
        waypoint_count: 3,
        quality: NavQuality::Degraded,
    }
}

fn sample(publication: Option<NavPublication>) -> wire::NavGuidanceState {
    match publication {
        Some(NavPublication::Sample(state)) => state,
        Some(NavPublication::Clear) => panic!("expected a guidance sample, got a clear"),
        None => panic!("expected a guidance sample, got nothing"),
    }
}

#[test]
fn every_sample_carries_the_navigation_role_under_the_host_clock() {
    let mut publisher = NavGuidancePublisher::for_session(SessionId::new(7));
    let state = sample(publisher.publication(Some(&guidance()), 1_000));
    let stamp = state.stamp.expect("a guidance sample is stamped");

    assert_eq!(stamp.role, wire::SourceRole::NavigationSolution as i32);
    assert_eq!(stamp.clock, wire::MeasurementClock::HostMonotonic as i32);
    assert_eq!(stamp.integrity, wire::SourceIntegrity::Unprotected as i32);
    assert_eq!(stamp.acquired_at_ns, 1_000);
    assert_eq!(stamp.source_epoch, 0);
    assert_eq!(stamp.source_incarnation.len(), 16);
}

#[test]
fn the_sequence_advances_once_per_sample_under_a_fixed_incarnation() {
    let mut publisher = NavGuidancePublisher::for_session(SessionId::new(7));
    let first = sample(publisher.publication(Some(&guidance()), 1_000));
    let second = sample(publisher.publication(Some(&guidance()), 2_000));
    let (first, second) = (
        first.stamp.expect("stamped"),
        second.stamp.expect("stamped"),
    );

    assert_eq!(first.sequence, 1);
    assert_eq!(second.sequence, 2);
    assert_eq!(
        first.source_incarnation, second.source_incarnation,
        "the attachment token is equality-only: it must not move under a live session"
    );
}

#[test]
fn a_different_session_is_a_different_incarnation() {
    let mut one = NavGuidancePublisher::for_session(SessionId::new(7));
    let mut other = NavGuidancePublisher::for_session(SessionId::new(8));
    let one = sample(one.publication(Some(&guidance()), 0));
    let other = sample(other.publication(Some(&guidance()), 0));

    assert_ne!(
        one.stamp.expect("stamped").source_incarnation,
        other.stamp.expect("stamped").source_incarnation
    );
}

#[test]
fn guidance_that_ends_is_cleared_exactly_once() {
    let mut publisher = NavGuidancePublisher::for_session(SessionId::new(7));
    assert!(
        publisher.publication(None, 0).is_none(),
        "nothing was ever published, so nothing is owed"
    );

    sample(publisher.publication(Some(&guidance()), 1_000));
    assert!(
        matches!(
            publisher.publication(None, 2_000),
            Some(NavPublication::Clear)
        ),
        "the end of guidance is announced"
    );
    assert!(
        publisher.publication(None, 3_000).is_none(),
        "the clear is owed once, not every tick"
    );
}

#[test]
fn field_values_survive_the_conversion() {
    let mut publisher = NavGuidancePublisher::for_session(SessionId::new(7));
    let state = sample(publisher.publication(Some(&guidance()), 0));

    assert_eq!(state.to_ident, "DEMOB");
    assert_eq!(state.from_ident, "DEMOA");
    assert!((state.course_rad - 1.25).abs() < 1e-6);
    assert!((state.lateral_deviation_m + 12.5).abs() < 1e-6);
    assert!((state.vertical_deviation_m - 3.0).abs() < 1e-6);
    assert!((state.distance_to_waypoint_m - 480.0).abs() < 1e-3);
    assert_eq!(state.leg_index, 1);
    assert_eq!(state.waypoint_count, 3);
    assert_eq!(state.solution_quality, 1);
}

#[test]
fn an_undefined_deviation_travels_as_nan_and_a_direct_to_leg_has_no_origin() {
    let mut publisher = NavGuidancePublisher::for_session(SessionId::new(7));
    let direct_to = NavGuidance {
        from_ident: None,
        lateral_deviation_m: None,
        vertical_deviation_m: None,
        quality: NavQuality::Unusable,
        ..guidance()
    };
    let state = sample(publisher.publication(Some(&direct_to), 0));

    assert!(state.from_ident.is_empty());
    assert!(
        state.lateral_deviation_m.is_nan(),
        "a direct-to leg has no cross-track reading, and zero would read as on-course"
    );
    assert!(state.vertical_deviation_m.is_nan());
    assert_eq!(state.solution_quality, 2);
}

#[test]
fn a_course_a_hair_under_north_narrows_inside_the_half_open_range() {
    let mut publisher = NavGuidancePublisher::for_session(SessionId::new(7));
    // A few f64 ULPs below 2π rounds UP to exactly f32 τ in the
    // narrowing; the wire contract is half-open [0, 2π), so due north
    // owns that boundary.
    let near_north = NavGuidance {
        course_rad: std::f64::consts::TAU - 1e-9,
        ..guidance()
    };
    let state = sample(publisher.publication(Some(&near_north), 0));
    assert_eq!(state.course_rad, 0.0, "the boundary folds to due north");
    assert!((0.0..std::f32::consts::TAU).contains(&state.course_rad));
}
