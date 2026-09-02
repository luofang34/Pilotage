#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use pilotage_trial::ConditionSet;
use sha2::{Digest as ShaDigest, Sha256};

use super::*;

#[path = "tests/stream.rs"]
mod stream_tests;
#[path = "tests/support.rs"]
mod support;

use support::{GOLDEN_RUN_SEED, cross_repository_digest, golden_condition};

#[test]
fn a_declaration_states_every_factor_the_condition_executes() {
    let condition = golden_condition();
    let declaration = declaration(&condition);

    assert_eq!(declaration.authority_scale_basis_points, 12_000);
    assert_eq!(declaration.hover_scale_basis_points, 9_000);
    assert_eq!(
        declaration.command_hold,
        Some(DeclaredCommandHold {
            fraction_basis_points: 100,
            decision_interval_samples: 100,
        })
    );
    assert_eq!(declaration.sensor_lanes.len(), 6);
    assert_eq!(
        declaration
            .required_capabilities
            .iter()
            .map(|capability| capability.as_str())
            .collect::<Vec<_>>(),
        vec![
            "actuator_authority",
            "command_hold",
            "hover_trim_uncertainty",
            "sensor_perturbation",
        ]
    );
}

#[test]
fn a_declared_amplitude_is_the_unit_the_executor_applies() {
    let condition = golden_condition();
    let declaration = declaration(&condition);

    // The artifact declares 0.02 gauss and 1.0 hectopascal; the controller
    // reads microtesla and pascal.
    let magnetometer = declaration.lane(8).expect("magnetometer z lane");
    let absolute_pressure = declaration.lane(9).expect("absolute pressure lane");

    assert!((f32::from_bits(magnetometer.peak_amplitude_bits) - 2.0).abs() < 1e-6);
    assert!((f32::from_bits(absolute_pressure.peak_amplitude_bits) - 100.0).abs() < 1e-4);
}

#[test]
fn the_declared_offset_is_the_one_the_contract_crate_derives() {
    let condition = golden_condition();
    let declaration = declaration(&condition);
    let references = condition
        .sensor_references_for_sample(GOLDEN_RUN_SEED, 123)
        .expect("contract references");

    for reference in references {
        let declared = declaration
            .lane(lane_tag(reference.lane()))
            .expect("declared lane");
        let derived = derivation::sensor_offset(
            declaration.condition_digest,
            declaration.run_seed,
            declared.lane_tag,
            reference.update_bucket(),
            f32::from_bits(declared.peak_amplitude_bits),
        );
        assert_eq!(
            derived.to_bits(),
            reference.offset().to_bits(),
            "lane {} offset",
            declared.lane_tag
        );
    }
}

#[test]
fn the_sensor_derivation_matches_the_cross_repository_golden() {
    // Accelerometer X, sample 10, interval 2, peak 2 m/s squared, raw 1.0.
    let offset = derivation::sensor_offset(cross_repository_digest(), GOLDEN_RUN_SEED, 0, 5, 2.0);
    assert_eq!((1.0_f32 + offset).to_bits(), 0x403c_4a40);

    // Differential pressure, sample 10, interval 1, peak 200 Pa, raw 500.0.
    let offset =
        derivation::sensor_offset(cross_repository_digest(), GOLDEN_RUN_SEED, 10, 10, 200.0);
    assert_eq!((500.0_f32 + offset).to_bits(), 0x43c4_6996);
}

#[test]
fn the_interval_identity_matches_the_cross_repository_golden() {
    let identity = derivation::interval_identity(
        cross_repository_digest(),
        GOLDEN_RUN_SEED,
        0x2122_2324_2526_2728,
        0x3132_3334_3536_3738,
        0x4142_4344_4546_4748,
    );

    assert_eq!(
        identity.to_string(),
        "94ab1093b990a952b30ec29395a88314347304a5b21ed170c862d2725d99bd6c"
    );
}

#[test]
fn the_hold_schedule_matches_the_one_the_contract_crate_derives() {
    let condition = golden_condition();
    let declaration = declaration(&condition);
    let hold = declaration.command_hold.expect("declared hold");

    let derived = derivation::hold_schedule(
        declaration.condition_digest,
        declaration.run_seed,
        2,
        3,
        1_001,
        hold,
    )
    .expect("hold schedule");
    let contract = condition
        .command_hold_decisions_for_interval(GOLDEN_RUN_SEED, 2, 3, 1_001)
        .expect("contract schedule");

    assert_eq!(derived, contract);
    assert_eq!(
        derived
            .iter()
            .enumerate()
            .filter(|(_, held)| **held)
            .map(|(position, _)| position)
            .collect::<Vec<_>>(),
        vec![89]
    );
}

#[test]
fn the_sensor_sample_identity_covers_its_own_preimage() {
    let mut values = [None; EXECUTED_SENSOR_LANE_COUNT];
    values[0] = Some(0x3f80_0000);
    values[3] = Some(0x4000_0000);
    let presence_mask = 0b1001_u16;

    let mut hasher = Sha256::new();
    hasher.update(b"aviate-effective-sensor-v1");
    hasher.update(presence_mask.to_le_bytes());
    for value in &values {
        hasher.update(value.unwrap_or(0_u32).to_le_bytes());
    }
    let expected: [u8; 32] = hasher.finalize().into();

    assert_eq!(
        derivation::sensor_sample_digest(presence_mask, &values),
        Digest::from_bytes(expected)
    );
}

#[test]
fn the_authority_scale_narrows_once_at_the_end() {
    // A scale taken wholly in the narrow domain gives another last bit, so
    // the wider intermediate is part of the contract rather than a detail.
    let lane = f32::from_bits(0x38d1_b717);
    let narrow = lane * (12_000.0 / 10_000.0);

    assert_eq!(
        derivation::scaled_authority(lane.to_bits(), 12_000),
        0x38fb_a882
    );
    assert_eq!(narrow.to_bits(), 0x38fb_a883);
}

#[test]
fn a_run_seed_separates_one_retry_from_the_execution_it_replaces() {
    let source = executed_run_seed(Digest::from_bytes([7; 32]));
    let replacement = executed_run_seed(Digest::from_bytes([8; 32]));

    assert_ne!(source, replacement);
    assert_eq!(source, executed_run_seed(Digest::from_bytes([7; 32])));
}

#[test]
fn a_declaration_that_omits_a_required_capability_is_refused() {
    let condition = golden_condition();
    let mut declaration = declaration(&condition);
    declaration.required_capabilities.remove(0);

    assert!(declaration.validate().is_err());
}

#[test]
fn a_declaration_whose_lanes_are_out_of_order_is_refused() {
    let condition = golden_condition();
    let mut declaration = declaration(&condition);
    declaration.sensor_lanes.swap(0, 1);

    assert!(declaration.validate().is_err());
}

#[test]
fn a_launch_that_names_one_value_for_two_identities_is_refused() {
    let condition = golden_condition();
    let declaration = declaration(&condition);

    assert!(
        ExecutedLaunchIdentity::new(
            Digest::from_bytes([1; 32]),
            declaration.condition_digest,
            declaration.condition_digest,
            declaration.run_seed,
            declaration.required_capabilities.clone(),
            3,
        )
        .is_err()
    );
}

fn declaration(condition: &ConditionSet) -> ExecutedUncertaintyDeclaration {
    ExecutedUncertaintyDeclaration::from_condition(
        condition,
        Digest::from_bytes([0xab; 32]),
        GOLDEN_RUN_SEED,
    )
    .expect("declaration")
}
