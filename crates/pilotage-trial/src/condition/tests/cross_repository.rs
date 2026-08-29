//! Values that this repository and the executor must both produce.
//!
//! Each expected value here comes from the written domain contract with an
//! independent digest implementation, not from recording what this code
//! returns. A change to a preimage, to the digest word, to the unit the
//! amplitude reaches the derivation in, or to the value mapping fails one of
//! these cases by value, on whichever side of the contract moved.
//!
//! The command-hold permutation word for the same identity is pinned with
//! the permutation itself, in the actuator tests.

#![allow(clippy::expect_used, clippy::panic)]

use super::super::{
    CommandHoldIntervalIdentity, CommandLossPolicy, SensorAxis, SensorNoiseLane,
    SensorNoiseReference, SensorReferenceLane,
};
use crate::Digest;

/// The identity inputs both repositories pin for these domains.
const GOLDEN_CONDITION_BYTES: [u8; 32] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31,
];
const GOLDEN_RUN_SEED: u64 = 0x1112_1314_1516_1718;
const GOLDEN_EPOCH: u64 = 0x2122_2324_2526_2728;
const GOLDEN_INDEX: u64 = 0x3132_3334_3536_3738;
const GOLDEN_FIRST_SEQUENCE: u64 = 0x4142_4344_4546_4748;

fn golden_digest() -> Digest {
    Digest::from_bytes(GOLDEN_CONDITION_BYTES)
}

fn golden_identity() -> CommandHoldIntervalIdentity {
    CommandHoldIntervalIdentity::new(
        golden_digest(),
        GOLDEN_RUN_SEED,
        GOLDEN_EPOCH,
        GOLDEN_INDEX,
        GOLDEN_FIRST_SEQUENCE,
    )
    .expect("interval identity")
}

#[test]
fn the_command_hold_interval_identity_matches_the_cross_repository_digest() {
    assert_eq!(
        golden_identity().digest().to_string(),
        "94ab1093b990a952b30ec29395a88314347304a5b21ed170c862d2725d99bd6c"
    );
}

#[test]
fn the_command_hold_schedule_matches_the_cross_repository_position() {
    let held = CommandLossPolicy::SeededZeroOrderHold {
        fraction_basis_points: 100,
        decision_interval_samples: 100,
    }
    .decisions_for_interval(
        CommandHoldIntervalIdentity::new(golden_digest(), GOLDEN_RUN_SEED, 0, 0, 1_001)
            .expect("interval identity"),
    )
    .expect("golden schedule")
    .iter()
    .enumerate()
    .filter_map(|(index, hold)| hold.then_some(index))
    .collect::<Vec<_>>();

    assert_eq!(held, vec![45]);
}

fn golden_reference(request: SensorNoiseLane, sample: u64) -> SensorNoiseReference {
    SensorNoiseReference::new(golden_digest(), GOLDEN_RUN_SEED, sample, request)
}

#[test]
fn the_accelerometer_lane_offset_matches_the_cross_repository_value() {
    let reference = golden_reference(
        SensorNoiseLane::Accelerometer {
            axis: SensorAxis::X,
            peak_amplitude_mps2: 2.0,
            update_interval_samples: 2,
        },
        10,
    );

    assert_eq!(reference.lane(), SensorReferenceLane::AccelerometerX);
    assert_eq!(reference.update_bucket(), 5);
    assert_eq!(reference.offset().to_bits(), 0x3ff8_9481);
    // The executor adds this offset to the raw lane value, so the sum is the
    // value the flight controller reads for a raw one meter per second
    // squared.
    assert_eq!((1.0_f32 + reference.offset()).to_bits(), 0x403c_4a40);
}

#[test]
fn the_differential_pressure_lane_converts_hectopascals_before_the_offset() {
    let reference = golden_reference(
        SensorNoiseLane::DifferentialPressure {
            peak_amplitude_hpa: 2.0,
            update_interval_samples: 1,
        },
        10,
    );

    assert_eq!(reference.lane(), SensorReferenceLane::DifferentialPressure);
    assert_eq!(reference.update_bucket(), 10);
    // Two hectopascals reach the derivation as two hundred pascals. An
    // amplitude left in the declared unit gives an offset one hundred times
    // smaller and fails here.
    assert_eq!(reference.offset().to_bits(), 0xc2d6_59a8);
    assert_eq!((500.0_f32 + reference.offset()).to_bits(), 0x43c4_6996);
}

#[test]
fn the_magnetometer_lane_converts_gauss_before_the_offset() {
    let reference = golden_reference(
        SensorNoiseLane::Magnetometer {
            axis: SensorAxis::X,
            peak_amplitude_gauss: 0.5,
            update_interval_samples: 4,
        },
        12,
    );

    assert_eq!(reference.lane(), SensorReferenceLane::MagnetometerX);
    assert_eq!(reference.update_bucket(), 3);
    // Half a gauss reaches the derivation as fifty microteslas.
    assert_eq!(reference.offset().to_bits(), 0x41ac_b0f2);
}

#[test]
fn a_changed_run_seed_or_condition_changes_every_lane_offset() {
    let request = SensorNoiseLane::Accelerometer {
        axis: SensorAxis::X,
        peak_amplitude_mps2: 2.0,
        update_interval_samples: 2,
    };
    let mut changed_bytes = GOLDEN_CONDITION_BYTES;
    changed_bytes[31] = GOLDEN_CONDITION_BYTES[31].wrapping_add(1);

    let changed_seed = SensorNoiseReference::new(
        golden_digest(),
        GOLDEN_RUN_SEED.wrapping_add(1),
        10,
        request,
    );
    let changed_condition = SensorNoiseReference::new(
        Digest::from_bytes(changed_bytes),
        GOLDEN_RUN_SEED,
        10,
        request,
    );

    assert_eq!(changed_seed.update_bucket(), 5);
    assert_eq!(changed_condition.update_bucket(), 5);
    assert_ne!(changed_seed.offset().to_bits(), 0x3ff8_9481);
    assert_ne!(changed_condition.offset().to_bits(), 0x3ff8_9481);
}

#[test]
fn the_executor_condition_document_decodes_and_re_encodes_byte_for_byte() {
    let fixture = include_bytes!("../../../fixtures/condition-v4.executor-golden.json");
    let fixture = fixture.strip_suffix(b"\n").unwrap_or(fixture);
    let value = super::super::ConditionSet::from_json(fixture).expect("executor condition");

    // The executor holds this exact document. Decoding and re-encoding it
    // without a byte of drift is what lets one artifact cross the launch
    // seam in either direction.
    assert_eq!(value.to_canonical_json().expect("canonical JSON"), fixture);
    assert_eq!(fixture.len(), 1_313);
    assert_eq!(
        value.canonical_digest().expect("digest").to_string(),
        "ecca66bfcc8bf95fd9bbdf663add8ae3f6aef2c765325d3bc3e288716f7ba763"
    );
}
