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
    AvailabilityProfile, AvailabilityProfileId, ExternalHealth, derive_inputs,
    health_from_integrity,
};
use crate::datum::{
    BaroSettingId, DatumRealizationId, GeodeticPosition, GeoidModelId, HorizontalDatum,
    LocalOriginId, TerrainRefId, VerticalDatum, VerticalPosition,
};
use crate::identity::{
    AttitudeQuality, CoherentSnapshot, IntegrityLevel, PositionQuality, SourceIncarnation,
    SourceStamp, StatedAttitude, StatedPosition,
};

// The simulator profile's limits, read through its public accessors — the SIM
// values these derivation tests build fresh/stale/inaccurate inputs around.
const SIM_FRESH_AGE_NS: u64 = AvailabilityProfile::simulator().fresh_age_ns();
const SIM_USABLE_AGE_NS: u64 = AvailabilityProfile::simulator().usable_age_ns();
const SIM_FRESH_POS_MM: u32 = AvailabilityProfile::simulator().fresh_pos_mm();
const SIM_FRESH_ATT_MRAD: u32 = AvailabilityProfile::simulator().fresh_att_mrad();
const SIM_USABLE_ATT_MRAD: u32 = AvailabilityProfile::simulator().usable_att_mrad();

/// Derives inputs under the simulator profile, whose limits are the SIM
/// allocation the derivation tests exercise.
fn derive_sim(
    position: &StatedPosition,
    attitude: &StatedAttitude,
    external: &ExternalHealth,
    reference_time: Epoch,
) -> SvsInputs {
    derive_inputs(
        position,
        attitude,
        external,
        reference_time,
        &AvailabilityProfile::simulator(),
    )
}

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
    let now = epoch(SIM_FRESH_AGE_NS / 2);
    let inputs = derive_sim(
        &position(IntegrityLevel::Trusted, 0),
        &attitude(IntegrityLevel::Trusted, 0),
        &external_ok(),
        now,
    );
    assert_eq!(SvsAvailability::assess(&inputs), SvsAvailability::Available);
}

#[test]
fn untrusted_position_can_never_be_available() {
    let now = epoch(SIM_FRESH_AGE_NS / 2);
    let inputs = derive_sim(
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
    let now = epoch(SIM_FRESH_AGE_NS / 2);
    let inputs = derive_sim(
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
    let inputs = derive_sim(
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
    let stale = derive_sim(
        &position(IntegrityLevel::Trusted, 0),
        &attitude(IntegrityLevel::Trusted, 0),
        &external_ok(),
        epoch(SIM_FRESH_AGE_NS + 1),
    );
    assert_eq!(
        SvsAvailability::assess(&stale),
        SvsAvailability::Degraded(AvailabilityReason::TimeCoherence),
    );
    let unusable = derive_sim(
        &position(IntegrityLevel::Trusted, 0),
        &attitude(IntegrityLevel::Trusted, 0),
        &external_ok(),
        epoch(SIM_USABLE_AGE_NS + 1),
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
    let inputs = derive_sim(
        &position(IntegrityLevel::Trusted, 0),
        &att,
        &external_ok(),
        epoch(SIM_FRESH_AGE_NS / 2),
    );
    assert_eq!(
        SvsAvailability::assess(&inputs),
        SvsAvailability::Unavailable(AvailabilityReason::TimeCoherence),
    );
}

#[test]
fn a_trusted_but_grossly_inaccurate_position_is_never_available() {
    // Trusted integrity, fresh, coherent — but the position 1-sigma is
    // u32::MAX mm. Accuracy participates: the scene is unavailable.
    let now = epoch(SIM_FRESH_AGE_NS / 2);
    let mut pos = position(IntegrityLevel::Trusted, 0);
    pos.quality = PositionQuality {
        horizontal_mm: u32::MAX,
        vertical_mm: 0,
    };
    let inputs = derive_sim(
        &pos,
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
fn a_trusted_but_imprecise_attitude_degrades_or_fails() {
    let now = epoch(SIM_FRESH_AGE_NS / 2);
    let mut att = attitude(IntegrityLevel::Trusted, 0);
    att.quality = AttitudeQuality {
        angular_mrad: SIM_FRESH_ATT_MRAD + 1,
    };
    let inputs = derive_sim(
        &position(IntegrityLevel::Trusted, 0),
        &att,
        &external_ok(),
        now,
    );
    assert_eq!(
        SvsAvailability::assess(&inputs),
        SvsAvailability::Degraded(AvailabilityReason::Attitude),
    );

    att.quality = AttitudeQuality {
        angular_mrad: SIM_USABLE_ATT_MRAD + 1,
    };
    let inputs = derive_sim(
        &position(IntegrityLevel::Trusted, 0),
        &att,
        &external_ok(),
        now,
    );
    assert_eq!(
        SvsAvailability::assess(&inputs),
        SvsAvailability::Unavailable(AvailabilityReason::Attitude),
    );
}

#[test]
fn position_quality_at_the_fresh_bound_is_still_available() {
    let now = epoch(SIM_FRESH_AGE_NS / 2);
    let mut pos = position(IntegrityLevel::Trusted, 0);
    pos.quality = PositionQuality {
        horizontal_mm: SIM_FRESH_POS_MM,
        vertical_mm: SIM_FRESH_POS_MM,
    };
    let inputs = derive_sim(
        &pos,
        &attitude(IntegrityLevel::Trusted, 0),
        &external_ok(),
        now,
    );
    assert_eq!(SvsAvailability::assess(&inputs), SvsAvailability::Available);
}

// ---- derivation under an explicit, traceable profile -----------------------

#[test]
fn the_same_reading_is_judged_by_the_selected_profile() {
    // A position 1-sigma that is fully fresh under the simulator profile but only
    // degraded under a stricter one: the verdict tracks the profile, not the
    // wire, so the same input yields different, deterministic results.
    let now = epoch(SIM_FRESH_AGE_NS / 2);
    let mut pos = position(IntegrityLevel::Trusted, 0);
    pos.quality = PositionQuality {
        horizontal_mm: 1_000,
        vertical_mm: 1_000,
    };

    let stricter = AvailabilityProfile::new(
        AvailabilityProfileId(2),
        1,
        SIM_FRESH_AGE_NS,
        SIM_USABLE_AGE_NS,
        500,
        2_000,
        SIM_FRESH_ATT_MRAD,
        SIM_USABLE_ATT_MRAD,
    )
    .expect("a monotonic profile");

    let under_sim = derive_sim(
        &pos,
        &attitude(IntegrityLevel::Trusted, 0),
        &external_ok(),
        now,
    );
    let under_strict = derive_inputs(
        &pos,
        &attitude(IntegrityLevel::Trusted, 0),
        &external_ok(),
        now,
        &stricter,
    );

    assert_eq!(
        SvsAvailability::assess(&under_sim),
        SvsAvailability::Available,
    );
    assert_eq!(
        SvsAvailability::assess(&under_strict),
        SvsAvailability::Degraded(AvailabilityReason::Position),
    );
}

#[test]
fn the_age_boundary_is_pinned_under_the_simulator_profile() {
    let att = attitude(IntegrityLevel::Trusted, 0);
    let pos = position(IntegrityLevel::Trusted, 0);
    let assess = |reference: Epoch| {
        SvsAvailability::assess(&derive_sim(&pos, &att, &external_ok(), reference))
    };
    // Exactly at the fresh age: still available.
    assert_eq!(assess(epoch(SIM_FRESH_AGE_NS)), SvsAvailability::Available);
    // One nanosecond older: degraded on time/coherence.
    assert_eq!(
        assess(epoch(SIM_FRESH_AGE_NS + 1)),
        SvsAvailability::Degraded(AvailabilityReason::TimeCoherence),
    );
    // Exactly at the usable age: still only degraded.
    assert_eq!(
        assess(epoch(SIM_USABLE_AGE_NS)),
        SvsAvailability::Degraded(AvailabilityReason::TimeCoherence),
    );
    // One nanosecond beyond usable: unavailable.
    assert_eq!(
        assess(epoch(SIM_USABLE_AGE_NS + 1)),
        SvsAvailability::Unavailable(AvailabilityReason::TimeCoherence),
    );
}
