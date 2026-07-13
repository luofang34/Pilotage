//! Tests for the deterministic, traceable availability verdict.
#![allow(clippy::expect_used, clippy::panic)]

use super::{AvailabilityReason, InputHealth, SvsAvailability, SvsInputs};

fn all_ok() -> SvsInputs {
    let ok = InputHealth::Ok;
    SvsInputs {
        position: ok,
        attitude: ok,
        integrity: ok,
        time_coherence: ok,
        calibration: ok,
        database: ok,
        coverage: ok,
        renderer: ok,
    }
}

#[test]
fn all_ok_is_available() {
    let v = SvsAvailability::assess(&all_ok());
    assert_eq!(v, SvsAvailability::Available);
    assert!(v.is_available());
    assert_eq!(v.reason(), AvailabilityReason::Nominal);
}

#[test]
fn nothing_known_is_unavailable_not_a_normal_scene() {
    let v = SvsAvailability::assess(&SvsInputs::all_failed());
    assert!(!v.is_available());
    assert!(matches!(v, SvsAvailability::Unavailable(_)));
}

#[test]
fn a_single_failed_input_makes_it_unavailable_for_that_reason() {
    let mut inputs = all_ok();
    inputs.calibration = InputHealth::Failed;
    assert_eq!(
        SvsAvailability::assess(&inputs),
        SvsAvailability::Unavailable(AvailabilityReason::Calibration),
    );
}

#[test]
fn a_single_degraded_input_degrades_for_that_reason() {
    let mut inputs = all_ok();
    inputs.coverage = InputHealth::Degraded;
    assert_eq!(
        SvsAvailability::assess(&inputs),
        SvsAvailability::Degraded(AvailabilityReason::Coverage),
    );
}

#[test]
fn failed_dominates_degraded_and_precedence_is_fixed() {
    let mut inputs = all_ok();
    // Renderer failed but position also degraded: a failure (unavailable)
    // dominates a degrade, and among the two the failed reason wins.
    inputs.position = InputHealth::Degraded;
    inputs.renderer = InputHealth::Failed;
    assert_eq!(
        SvsAvailability::assess(&inputs),
        SvsAvailability::Unavailable(AvailabilityReason::Renderer),
    );

    // Two failures: the higher-precedence one (position before integrity) wins,
    // deterministically.
    let mut two = all_ok();
    two.integrity = InputHealth::Failed;
    two.position = InputHealth::Failed;
    assert_eq!(
        SvsAvailability::assess(&two),
        SvsAvailability::Unavailable(AvailabilityReason::Position),
    );
}

#[test]
fn assessment_is_deterministic() {
    let mut inputs = all_ok();
    inputs.time_coherence = InputHealth::Degraded;
    inputs.database = InputHealth::Failed;
    let first = SvsAvailability::assess(&inputs);
    let second = SvsAvailability::assess(&inputs);
    assert_eq!(first, second);
    assert_eq!(
        first,
        SvsAvailability::Unavailable(AvailabilityReason::Database)
    );
}

#[test]
fn input_health_decodes_fail_closed() {
    assert_eq!(InputHealth::from_u8_fail_closed(0), InputHealth::Ok);
    assert_eq!(InputHealth::from_u8_fail_closed(1), InputHealth::Degraded);
    assert_eq!(InputHealth::from_u8_fail_closed(2), InputHealth::Failed);
    assert_eq!(
        InputHealth::from_u8_fail_closed(200),
        InputHealth::Failed,
        "an unknown health is failed, never ok"
    );
}

#[test]
fn availability_reason_wire_codes_round_trip_and_reject_unknown() {
    for r in [
        AvailabilityReason::Nominal,
        AvailabilityReason::Position,
        AvailabilityReason::Attitude,
        AvailabilityReason::Integrity,
        AvailabilityReason::TimeCoherence,
        AvailabilityReason::Calibration,
        AvailabilityReason::Database,
        AvailabilityReason::Coverage,
        AvailabilityReason::Renderer,
    ] {
        assert_eq!(AvailabilityReason::from_u8(r.to_u8()), Some(r));
    }
    assert_eq!(AvailabilityReason::from_u8(200), None);
}
