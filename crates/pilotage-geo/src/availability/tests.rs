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

// ---- derivation from validated inputs --------------------------------------

use pilotage_frames::{ClockDomain, Epoch, Quat, TimeScale};

use super::{
    ExternalHealth, MAX_FRESH_AGE_NS, MAX_USABLE_AGE_NS, derive_inputs, health_from_integrity,
};
use crate::datum::{
    BaroSettingId, DatumRealizationId, GeodeticPosition, GeoidModelId, HorizontalDatum,
    LocalOriginId, TerrainRefId, VerticalDatum, VerticalPosition,
};
use crate::identity::{
    AttitudeQuality, CoherentSnapshot, IntegrityLevel, PositionQuality, SourceIncarnation,
    SourceStamp, StatedAttitude, StatedPosition,
};

fn epoch(nanos: u64) -> Epoch {
    Epoch {
        clock: ClockDomain::Simulation,
        scale: TimeScale::Monotonic,
        nanos,
    }
}

fn stamp(integrity: IntegrityLevel, nanos: u64) -> SourceStamp {
    SourceStamp {
        source_id: 1,
        incarnation: SourceIncarnation([1; 16]),
        generation: 0,
        sequence: 0,
        acquired_at: epoch(nanos),
        integrity,
        snapshot: CoherentSnapshot {
            producer: SourceIncarnation([9; 16]),
            generation: 3,
            id: 77,
        },
    }
}

fn position(integrity: IntegrityLevel, nanos: u64) -> StatedPosition {
    let vertical = VerticalPosition::new(
        100.0,
        VerticalDatum::Ellipsoid,
        GeoidModelId::UNDECLARED,
        TerrainRefId::UNDECLARED,
        BaroSettingId::UNDECLARED,
        LocalOriginId::UNDECLARED,
    )
    .expect("ellipsoid");
    StatedPosition {
        position: GeodeticPosition::new(
            10.0,
            20.0,
            HorizontalDatum::Wgs84,
            DatumRealizationId::UNDECLARED,
            vertical,
        )
        .expect("position"),
        stamp: stamp(integrity, nanos),
        quality: PositionQuality {
            horizontal_mm: 1000,
            vertical_mm: 2000,
        },
    }
}

fn attitude(integrity: IntegrityLevel, nanos: u64) -> StatedAttitude {
    StatedAttitude {
        attitude: Quat::IDENTITY,
        stamp: stamp(integrity, nanos),
        quality: AttitudeQuality { angular_mrad: 5 },
    }
}

fn external_ok() -> ExternalHealth {
    let ok = InputHealth::Ok;
    ExternalHealth {
        integrity: ok,
        calibration: ok,
        database: ok,
        coverage: ok,
        renderer: ok,
    }
}

#[test]
fn health_from_integrity_only_trusts_the_top_level() {
    assert_eq!(
        health_from_integrity(IntegrityLevel::Trusted),
        InputHealth::Ok
    );
    assert_eq!(
        health_from_integrity(IntegrityLevel::Monitored),
        InputHealth::Degraded
    );
    assert_eq!(
        health_from_integrity(IntegrityLevel::Untrusted),
        InputHealth::Failed
    );
    assert_eq!(
        health_from_integrity(IntegrityLevel::Unknown),
        InputHealth::Failed,
        "an unmonitored reading is not trusted for a scene"
    );
}

#[test]
fn fresh_trusted_coherent_inputs_are_available() {
    let now = epoch(MAX_FRESH_AGE_NS / 2);
    let inputs = derive_inputs(
        &position(IntegrityLevel::Trusted, 0),
        &attitude(IntegrityLevel::Trusted, 0),
        &external_ok(),
        now,
    );
    assert_eq!(SvsAvailability::assess(&inputs), SvsAvailability::Available);
}

#[test]
fn untrusted_position_can_never_be_available() {
    let now = epoch(MAX_FRESH_AGE_NS / 2);
    let inputs = derive_inputs(
        &position(IntegrityLevel::Untrusted, 0),
        &attitude(IntegrityLevel::Trusted, 0),
        &external_ok(),
        now,
    );
    assert_eq!(
        SvsAvailability::assess(&inputs),
        SvsAvailability::Unavailable(AvailabilityReason::Position),
    );
}

#[test]
fn monitored_attitude_degrades_the_scene() {
    let now = epoch(MAX_FRESH_AGE_NS / 2);
    let inputs = derive_inputs(
        &position(IntegrityLevel::Trusted, 0),
        &attitude(IntegrityLevel::Monitored, 0),
        &external_ok(),
        now,
    );
    assert_eq!(
        SvsAvailability::assess(&inputs),
        SvsAvailability::Degraded(AvailabilityReason::Attitude),
    );
}

#[test]
fn a_future_sample_fails_time_coherence() {
    // Reference time earlier than acquisition → future sample → failed.
    let inputs = derive_inputs(
        &position(IntegrityLevel::Trusted, 1_000_000),
        &attitude(IntegrityLevel::Trusted, 1_000_000),
        &external_ok(),
        epoch(0),
    );
    assert_eq!(
        SvsAvailability::assess(&inputs),
        SvsAvailability::Unavailable(AvailabilityReason::TimeCoherence),
    );
}

#[test]
fn a_stale_pair_degrades_and_an_unusable_pair_fails() {
    let stale = derive_inputs(
        &position(IntegrityLevel::Trusted, 0),
        &attitude(IntegrityLevel::Trusted, 0),
        &external_ok(),
        epoch(MAX_FRESH_AGE_NS + 1),
    );
    assert_eq!(
        SvsAvailability::assess(&stale),
        SvsAvailability::Degraded(AvailabilityReason::TimeCoherence),
    );
    let unusable = derive_inputs(
        &position(IntegrityLevel::Trusted, 0),
        &attitude(IntegrityLevel::Trusted, 0),
        &external_ok(),
        epoch(MAX_USABLE_AGE_NS + 1),
    );
    assert_eq!(
        SvsAvailability::assess(&unusable),
        SvsAvailability::Unavailable(AvailabilityReason::TimeCoherence),
    );
}

#[test]
fn incoherent_position_and_attitude_fail_time_coherence() {
    let mut att = attitude(IntegrityLevel::Trusted, 0);
    att.stamp.snapshot.id = 78; // a different snapshot instance
    let inputs = derive_inputs(
        &position(IntegrityLevel::Trusted, 0),
        &att,
        &external_ok(),
        epoch(MAX_FRESH_AGE_NS / 2),
    );
    assert_eq!(
        SvsAvailability::assess(&inputs),
        SvsAvailability::Unavailable(AvailabilityReason::TimeCoherence),
    );
}
