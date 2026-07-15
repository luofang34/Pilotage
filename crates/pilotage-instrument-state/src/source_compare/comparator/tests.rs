#![allow(clippy::expect_used, clippy::panic)]

use super::SourceComparator;
use crate::source_compare::{
    AirframeSourcePolicy, Candidate, IntegrityLevel, ScalarMeasure, ScalarUnit, SourceEpoch,
    SourceId,
};
use pilotage_alerts::MiscompareFault;

fn air(source: u8, now: u64, value: f32) -> Candidate<ScalarMeasure> {
    Candidate {
        source: SourceId(source),
        epoch: SourceEpoch(1),
        source_time_ms: now,
        receive_time_ms: now,
        sequence: now as u32,
        valid: true,
        integrity: IntegrityLevel::None,
        accuracy: 0.0,
        measurement: ScalarMeasure {
            value,
            unit: ScalarUnit::MetersPerSecond,
        },
    }
}

/// A single-source airspeed candidate with an explicit sequence, fresh at `now`.
fn seq(sequence: u32, now: u64) -> Candidate<ScalarMeasure> {
    Candidate {
        sequence,
        ..air(1, now, 100.0)
    }
}

/// Whether a lone source-1 sample with this sequence survives the usability
/// gate: a usable sample selects its source, a dropped one leaves nothing to
/// select.
fn accepted(c: &mut SourceComparator, sequence: u32, now: u64) -> bool {
    c.step(&[seq(sequence, now)], &pol(), now).selected == Some(SourceId(1))
}

fn pol() -> AirframeSourcePolicy {
    AirframeSourcePolicy::simulator(MiscompareFault::Airspeed)
}

#[test]
fn sequence_wrap_is_a_normal_advance_not_a_replay() {
    // Producers count with wrapping_add(1), so u32::MAX → 0 is the next
    // sample in order and must stay usable across the wrap.
    let mut c = SourceComparator::new(MiscompareFault::Airspeed);
    assert!(accepted(&mut c, u32::MAX - 1, 0));
    assert!(accepted(&mut c, u32::MAX, 100));
    assert!(
        accepted(&mut c, 0, 200),
        "u32::MAX → 0 is one wrapping step"
    );
    assert!(accepted(&mut c, 1, 300), "and the count keeps advancing");
}

#[test]
fn duplicate_sequence_is_dropped_as_replay() {
    let mut c = SourceComparator::new(MiscompareFault::Airspeed);
    assert!(accepted(&mut c, 7, 0));
    assert!(!accepted(&mut c, 7, 100), "delta 0 is a replayed sample");
    assert!(
        accepted(&mut c, 8, 200),
        "the stream recovers on the next advance"
    );
}

#[test]
fn backward_sequence_is_dropped_as_reorder() {
    let mut c = SourceComparator::new(MiscompareFault::Airspeed);
    assert!(accepted(&mut c, 7, 0));
    assert!(
        !accepted(&mut c, 5, 100),
        "a backward delta lands in the rejected half range"
    );
    assert!(
        !accepted(&mut c, 7, 200),
        "the high-water mark did not regress to the dropped sample"
    );
    assert!(accepted(&mut c, 8, 300));
}

#[test]
fn half_range_sequence_step_is_ambiguous_and_dropped() {
    // A delta of exactly 0x8000_0000 is equally far forward and backward, so
    // it is rejected; one less is the largest accepted forward jump.
    let mut c = SourceComparator::new(MiscompareFault::Airspeed);
    assert!(accepted(&mut c, 0, 0));
    assert!(
        !accepted(&mut c, 0x8000_0000, 100),
        "exact half range is ambiguous"
    );
    let mut c2 = SourceComparator::new(MiscompareFault::Airspeed);
    assert!(accepted(&mut c2, 0, 0));
    assert!(
        accepted(&mut c2, 0x7FFF_FFFF, 100),
        "just under the half range is a forward advance"
    );
}

#[test]
fn generation_wraps_without_panicking() {
    let p = AirframeSourcePolicy::simulator(MiscompareFault::Airspeed);
    let mut c = SourceComparator::new(MiscompareFault::Airspeed);
    // Force the wrapping boundary: the next identity change must roll to 0
    // rather than overflow-panic in a debug build.
    c.generation = u32::MAX;
    let first = c.step(&[air(1, 0, 100.0), air(2, 0, 100.0)], &p, 0);
    assert_eq!(first.generation, 0, "wrapping_add(1) on u32::MAX is 0");

    // A steady step with no identity change holds the counter.
    let steady = c.step(&[air(1, 1, 100.0), air(2, 1, 100.0)], &p, 1);
    assert_eq!(steady.generation, 0);

    // Losing the peer changes the comparison state, advancing the counter.
    let changed = c.step(&[air(1, 2, 100.0)], &p, 2);
    assert_eq!(changed.generation, 1);
}
