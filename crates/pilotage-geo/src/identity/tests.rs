//! Tests for source identity, age, and coherence.
#![allow(clippy::expect_used, clippy::panic)]

use pilotage_frames::{ClockDomain, Epoch, TimeScale};

use super::{Accuracy, IntegrityLevel, SnapshotId, SourceIncarnation, SourceStamp};

fn epoch(clock: ClockDomain, scale: TimeScale, nanos: u64) -> Epoch {
    Epoch {
        clock,
        scale,
        nanos,
    }
}

fn stamp(acquired_at: Epoch, snapshot: SnapshotId) -> SourceStamp {
    SourceStamp {
        source_id: 1,
        incarnation: SourceIncarnation([7; 16]),
        generation: 0,
        sequence: 0,
        acquired_at,
        integrity: IntegrityLevel::Trusted,
        accuracy: Accuracy {
            horizontal_mm: 1000,
            vertical_mm: 2000,
        },
        snapshot,
    }
}

#[test]
fn age_is_computed_only_within_one_clock_and_scale() {
    let acq = epoch(ClockDomain::Gnss, TimeScale::Gps, 1_000);
    let s = stamp(acq, SnapshotId::NONE);
    let same = epoch(ClockDomain::Gnss, TimeScale::Gps, 3_000);
    assert_eq!(s.age_ns(same), Some(2_000));
    // Earlier `now` saturates at zero rather than underflowing.
    assert_eq!(
        s.age_ns(epoch(ClockDomain::Gnss, TimeScale::Gps, 500)),
        Some(0)
    );
}

#[test]
fn age_across_clock_domains_is_never_inferred() {
    let s = stamp(
        epoch(ClockDomain::Gnss, TimeScale::Gps, 1_000),
        SnapshotId::NONE,
    );
    assert_eq!(
        s.age_ns(epoch(ClockDomain::VehicleBoot, TimeScale::Gps, 3_000)),
        None,
        "different clock domain: no age"
    );
    assert_eq!(
        s.age_ns(epoch(ClockDomain::Gnss, TimeScale::Utc, 3_000)),
        None,
        "different time scale: no age"
    );
}

#[test]
fn coherence_requires_a_declared_matching_snapshot() {
    let acq = epoch(ClockDomain::Gnss, TimeScale::Gps, 1_000);
    let a = stamp(acq, SnapshotId(42));
    let b = stamp(acq, SnapshotId(42));
    let c = stamp(acq, SnapshotId(43));
    let undeclared = stamp(acq, SnapshotId::NONE);
    assert!(
        a.coherent_with(&b),
        "matching declared snapshot is coherent"
    );
    assert!(!a.coherent_with(&c), "different snapshot is not coherent");
    assert!(
        !undeclared.coherent_with(&undeclared),
        "an undeclared snapshot is never coherent, even with itself"
    );
}

#[test]
fn integrity_wire_codes_round_trip_and_reject_unknown() {
    for level in [
        IntegrityLevel::Unknown,
        IntegrityLevel::Untrusted,
        IntegrityLevel::Monitored,
        IntegrityLevel::Trusted,
    ] {
        assert_eq!(IntegrityLevel::from_u8(level.to_u8()), Some(level));
    }
    assert_eq!(IntegrityLevel::from_u8(9), None);
    assert!(IntegrityLevel::Trusted.is_trusted());
    assert!(!IntegrityLevel::Monitored.is_trusted());
}
