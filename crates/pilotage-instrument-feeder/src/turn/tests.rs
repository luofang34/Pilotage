//! Turn-derivation bounds, stream discipline, and circular differencing.

#![allow(clippy::expect_used, clippy::panic)]

use core::f64::consts::PI;

use super::{MAX_TURN_DT_MS, MIN_TURN_DT_MS, TurnDerivation};
use crate::stamp::{CLOCK_SIMULATION, ROLE_OPERATIONAL_ESTIMATE, RawStamp};

fn stamp(sequence: u32, at_ms: f64) -> RawStamp {
    RawStamp {
        role: ROLE_OPERATIONAL_ESTIMATE,
        integrity: 2,
        source_id: 7,
        incarnation: [1; 16],
        epoch: 1,
        sequence,
        acquired_at_ns: (at_ms * 1.0e6) as u64,
        clock: CLOCK_SIMULATION,
    }
}

#[test]
fn a_rate_is_derived_only_within_the_dt_bounds() {
    let mut turn = TurnDerivation::new();
    assert!(turn.update(0.0, 5.0, Some(&stamp(1, 0.0))).is_none());
    // 100 ms between samples, 0.1 rad — 1 rad/s.
    let decl = turn
        .update(0.1, 5.0, Some(&stamp(2, 100.0)))
        .expect("in-bounds dt declares");
    assert!((decl.turn_rps - 1.0).abs() < 1e-5);

    // Below the minimum dt: seeds but declares nothing.
    assert!(
        turn.update(0.2, 5.0, Some(&stamp(3, 100.0 + MIN_TURN_DT_MS / 2.0)))
            .is_none()
    );
    // Beyond the maximum dt: also nothing.
    let mut cold = TurnDerivation::new();
    assert!(cold.update(0.0, 5.0, Some(&stamp(1, 0.0))).is_none());
    assert!(
        cold.update(0.1, 5.0, Some(&stamp(2, MAX_TURN_DT_MS + 100.0)))
            .is_none()
    );
}

#[test]
fn the_difference_is_circular_into_half_open_pi() {
    let mut turn = TurnDerivation::new();
    // 359 deg -> 1 deg over 100 ms is +2 deg, never -358.
    let start = 359.0_f64.to_radians();
    let end = 1.0_f64.to_radians();
    assert!(turn.update(start, 5.0, Some(&stamp(1, 0.0))).is_none());
    let decl = turn
        .update(end, 5.0, Some(&stamp(2, 100.0)))
        .expect("declares");
    let expected = 2.0_f64.to_radians() / 0.1;
    assert!((decl.turn_rps - expected).abs() < 1e-4, "{}", decl.turn_rps);
    assert!(decl.turn_rps.abs() < PI / 0.1);
}

#[test]
fn repeats_redeclare_and_reordered_samples_declare_nothing() {
    let mut turn = TurnDerivation::new();
    assert!(turn.update(0.0, 5.0, Some(&stamp(1, 0.0))).is_none());
    turn.update(0.1, 5.0, Some(&stamp(2, 100.0)))
        .expect("declares");
    // Same sequence: re-declare the cached rate with the fresh age.
    let repeat = turn
        .update(0.1, 42.0, Some(&stamp(2, 100.0)))
        .expect("re-declares");
    assert_eq!(repeat.age_ms, 42.0);
    // Serially older: ignored entirely.
    assert!(turn.update(0.05, 5.0, Some(&stamp(1, 50.0))).is_none());
    // The cached rate still re-declares afterward.
    assert!(turn.update(0.1, 43.0, Some(&stamp(2, 100.0))).is_some());
}

#[test]
fn stream_changes_and_gaps_reset_the_state() {
    let mut turn = TurnDerivation::new();
    assert!(turn.update(0.0, 5.0, Some(&stamp(1, 0.0))).is_none());
    turn.update(0.1, 5.0, Some(&stamp(2, 100.0)))
        .expect("declares");
    // A missing heading resets: the next sample only seeds.
    assert!(turn.update(f64::NAN, 5.0, Some(&stamp(3, 200.0))).is_none());
    assert!(turn.update(0.2, 5.0, Some(&stamp(4, 300.0))).is_none());

    // An epoch change is a different stream: reset then seed.
    let mut moved = stamp(5, 400.0);
    moved.epoch = 2;
    assert!(turn.update(0.3, 5.0, Some(&moved)).is_none());
    let mut next = stamp(6, 500.0);
    next.epoch = 2;
    assert!(turn.update(0.4, 5.0, Some(&next)).is_some());
}
