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
