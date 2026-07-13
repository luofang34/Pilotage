//! Tests for source identity, typed age, and coherent-snapshot binding.
#![allow(clippy::expect_used, clippy::panic)]

use pilotage_frames::{ClockDomain, Epoch, TimeScale};

use super::{CoherentSnapshot, IntegrityLevel, SourceIncarnation, SourceStamp};
use crate::error::AgeError;

fn epoch(clock: ClockDomain, scale: TimeScale, nanos: u64) -> Epoch {
    Epoch {
        clock,
        scale,
        nanos,
    }
}

fn snap(producer: u8, generation: u32, id: u64) -> CoherentSnapshot {
    CoherentSnapshot {
        producer: SourceIncarnation([producer; 16]),
        generation,
        id,
    }
}

fn stamp(acquired_at: Epoch, snapshot: CoherentSnapshot) -> SourceStamp {
    SourceStamp {
        source_id: 1,
        incarnation: SourceIncarnation([7; 16]),
        generation: 0,
        sequence: 0,
        acquired_at,
        integrity: IntegrityLevel::Trusted,
        snapshot,
    }
}

#[test]
fn age_is_computed_only_within_one_clock_and_scale() {
    let acq = epoch(ClockDomain::Gnss, TimeScale::Gps, 1_000);
    let s = stamp(acq, CoherentSnapshot::NONE);
    let same = epoch(ClockDomain::Gnss, TimeScale::Gps, 3_000);
    assert_eq!(s.age_ns(same), Ok(2_000));
}

#[test]
fn a_future_sample_is_a_typed_error_never_a_saturated_zero() {
    let s = stamp(
        epoch(ClockDomain::Gnss, TimeScale::Gps, 1_000),
        CoherentSnapshot::NONE,
    );
    let earlier = epoch(ClockDomain::Gnss, TimeScale::Gps, 500);
    assert_eq!(
        s.age_ns(earlier),
        Err(AgeError::FutureSample {
            acquired_nanos: 1_000,
            now_nanos: 500,
        }),
        "a sample from the future must not read as age zero"
    );
}

#[test]
fn age_across_clock_domains_is_never_inferred() {
    let s = stamp(
        epoch(ClockDomain::Gnss, TimeScale::Gps, 1_000),
        CoherentSnapshot::NONE,
    );
    assert_eq!(
        s.age_ns(epoch(ClockDomain::VehicleBoot, TimeScale::Gps, 3_000)),
        Err(AgeError::ClockMismatch),
        "different clock domain: no age"
    );
    assert_eq!(
        s.age_ns(epoch(ClockDomain::Gnss, TimeScale::Utc, 3_000)),
        Err(AgeError::ScaleMismatch),
        "different time scale: no age"
    );
}

#[test]
fn coherence_binds_the_full_snapshot_identity_and_time_base() {
    let acq = epoch(ClockDomain::Gnss, TimeScale::Gps, 1_000);
    let a = stamp(acq, snap(1, 5, 42));
    let b = stamp(acq, snap(1, 5, 42));
    assert!(
        a.coherent_with(&b),
        "same producer/generation/id is coherent"
    );

    // Same numeric id but a different producer is a different snapshot.
    assert!(
        !a.coherent_with(&stamp(acq, snap(2, 5, 42))),
        "equal id from a different producer is not coherent"
    );
    // Same numeric id but a different generation is a different snapshot.
    assert!(
        !a.coherent_with(&stamp(acq, snap(1, 6, 42))),
        "equal id from a different generation is not coherent"
    );
    // Same snapshot identity but a different time base is not coherent.
    let other_clock = epoch(ClockDomain::Simulation, TimeScale::Monotonic, 1_000);
    assert!(
        !a.coherent_with(&stamp(other_clock, snap(1, 5, 42))),
        "a different clock/scale is not one coherent sampling"
    );
}

#[test]
fn an_undeclared_snapshot_is_never_coherent() {
    let acq = epoch(ClockDomain::Gnss, TimeScale::Gps, 1_000);
    let undeclared = stamp(acq, CoherentSnapshot::NONE);
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
