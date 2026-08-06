//! Guidance pinning, replay safety, and well-formedness.

#![allow(clippy::expect_used, clippy::panic)]

use pilotage_instrument_state::IdentStr;

use super::{Guidance, NavGuidanceTracker, NavReject};
use crate::stamp::{CLOCK_HOST_MONOTONIC, ROLE_NAVIGATION_SOLUTION, RawStamp};

fn stamp(epoch: u32, sequence: u32) -> RawStamp {
    RawStamp {
        role: ROLE_NAVIGATION_SOLUTION,
        integrity: 2,
        source_id: 11,
        incarnation: [2; 16],
        epoch,
        sequence,
        acquired_at_ns: 0,
        clock: CLOCK_HOST_MONOTONIC,
    }
}

fn guidance(course_rad: f32) -> Guidance {
    Guidance {
        to_ident: IdentStr::new("WPT-1").expect("valid"),
        from_ident: IdentStr::EMPTY,
        course_rad,
        lateral_deviation_m: 10.0,
        vertical_deviation_m: f32::NAN,
        distance_to_waypoint_m: 900.0,
        leg_index: 1,
        waypoint_count: 4,
        solution_quality: 0,
    }
}

#[test]
fn only_new_samples_restart_the_age_clock() {
    let mut tracker = NavGuidanceTracker::new();
    assert!(tracker.observe(None, 0.0).is_none());
    let snap = tracker
        .observe(Some(&(stamp(1, 1), guidance(0.5))), 0.0)
        .expect("accepted");
    assert_eq!(snap.age_ms, 0.0);
    // A duplicate never refreshes.
    let snap = tracker
        .observe(Some(&(stamp(1, 1), guidance(0.9))), 500.0)
        .expect("snapshot stands");
    assert_eq!(snap.age_ms, 500.0);
    assert!((snap.guidance.course_rad - 0.5).abs() < 1e-6);
    let (counters, last) = tracker.diagnostics();
    assert_eq!(counters.duplicates, 1);
    assert_eq!(last, Some(NavReject::Duplicate));
}

#[test]
fn identity_is_pinned_and_malformed_guidance_is_refused() {
    let mut tracker = NavGuidanceTracker::new();
    tracker
        .observe(Some(&(stamp(1, 1), guidance(0.5))), 0.0)
        .expect("accepted");
    let mut foreign = stamp(2, 2);
    foreign.source_id = 99;
    tracker.observe(Some(&(foreign, guidance(1.0))), 1.0);
    let (counters, _) = tracker.diagnostics();
    assert_eq!(counters.wrong_source, 1);

    tracker.observe(Some(&(stamp(1, 2), guidance(f32::NAN))), 2.0);
    let (counters, last) = tracker.diagnostics();
    assert_eq!(counters.malformed_guidance, 1);
    assert_eq!(last, Some(NavReject::MalformedGuidance));

    // NaN deviations are the schema's "not tracking" and stay legal.
    let mut untracked = guidance(1.5);
    untracked.lateral_deviation_m = f32::NAN;
    let snap = tracker
        .observe(Some(&(stamp(1, 2), untracked)), 3.0)
        .expect("accepted");
    assert!((snap.guidance.course_rad - 1.5).abs() < 1e-6);
}

#[test]
fn epoch_advances_restart_and_replays_are_refused() {
    let mut tracker = NavGuidanceTracker::new();
    tracker
        .observe(Some(&(stamp(5, 9), guidance(0.5))), 0.0)
        .expect("accepted");
    let snap = tracker
        .observe(Some(&(stamp(6, 1), guidance(0.7))), 1.0)
        .expect("accepted");
    assert!((snap.guidance.course_rad - 0.7).abs() < 1e-6);
    tracker.observe(Some(&(stamp(5, 10), guidance(0.9))), 2.0);
    let (counters, _) = tracker.diagnostics();
    assert_eq!(counters.duplicates, 1);
}
