#![allow(clippy::expect_used, clippy::panic)]

use super::*;

fn non_nominal() -> ConditionSet {
    let mut value = condition(42);
    value.sensor = SensorCondition::BoundedNoise {
        lanes: vec![SensorNoiseLane::PressureAltitude {
            peak_amplitude_m: 2.0,
            update_interval_samples: 10,
        }],
    };
    value.actuator = ActuatorCondition {
        authority_scale_basis_points: 8_000,
        command_loss: CommandLossPolicy::SeededZeroOrderHold {
            fraction_basis_points: 100,
            decision_interval_samples: 100,
        },
    };
    value.controller_initialization.hover_thrust_force =
        HoverThrustForceInitialization::ScaleBaseline {
            scale_basis_points: 9_000,
        };
    value
}

#[test]
fn a_nominal_condition_needs_no_uncertainty_capability() {
    let value = condition(42);

    assert!(value.required_capabilities().is_empty());
    value
        .validate_capability_report(&[], HoverEstimatorMode::Online)
        .expect("a nominal condition keeps the current behavior");
}

#[test]
fn capability_validation_is_exact_and_needs_an_inactive_hover_estimator() {
    let value = non_nominal();
    let capabilities = value.required_capabilities();

    assert_eq!(
        capabilities,
        vec![
            BackendCapability::SensorPerturbation,
            BackendCapability::ActuatorAuthority,
            BackendCapability::CommandHold,
            BackendCapability::HoverTrimUncertainty,
        ]
    );
    for absent in 0..capabilities.len() {
        let mut partial = capabilities.clone();
        partial.remove(absent);
        assert!(matches!(
            value.validate_capability_report(&partial, HoverEstimatorMode::Frozen),
            Err(ValidationError::UnsupportedConditionCapability { .. })
        ));
    }
    assert!(matches!(
        value.validate_capability_report(&capabilities, HoverEstimatorMode::Online),
        Err(ValidationError::ActiveHoverEstimator { .. })
    ));
    value
        .validate_capability_report(&capabilities, HoverEstimatorMode::Disabled)
        .expect("disabled estimator");
    value
        .validate_capability_report(&capabilities, HoverEstimatorMode::Frozen)
        .expect("frozen estimator");
}

#[test]
fn a_static_frozen_mode_cannot_admit_a_live_online_runtime() {
    let value = non_nominal();
    let capabilities = value.required_capabilities();

    value
        .validate_capability_report(&capabilities, HoverEstimatorMode::Frozen)
        .expect("preparation admits the known declaration");
    assert!(matches!(
        value.validate_capability_report(&capabilities, HoverEstimatorMode::Online),
        Err(ValidationError::ActiveHoverEstimator { .. })
    ));
}

#[test]
fn a_backend_declaration_carries_its_own_estimator_mode() {
    let value = non_nominal();
    let mut backend = crate::BackendCapabilities {
        schema_version: crate::BACKEND_CAPABILITIES_SCHEMA_VERSION,
        backend: crate::ArtifactIdentity {
            id: "reference-backend".to_owned(),
            revision: "r1".to_owned(),
            digest: Digest::from_bytes([5; 32]),
        },
        capabilities: value.required_capabilities(),
        hover_estimator_mode: HoverEstimatorMode::Frozen,
    };

    value
        .validate_for_backend(&backend)
        .expect("frozen backend");
    backend.hover_estimator_mode = HoverEstimatorMode::Online;
    assert!(matches!(
        value.validate_for_backend(&backend),
        Err(ValidationError::ActiveHoverEstimator { .. })
    ));
}

#[test]
fn a_hover_force_request_stays_outside_the_plant_and_the_estimator() {
    let value = non_nominal();

    // The hover force is the controller feed-forward input, so it needs the
    // hover-trim capability and never the actuator-authority capability.
    assert!(
        value
            .controller_initialization
            .required_capabilities()
            .contains(&BackendCapability::HoverTrimUncertainty)
    );
    assert!(
        !value
            .controller_initialization
            .required_capabilities()
            .contains(&BackendCapability::ActuatorAuthority)
    );
    assert!(
        !value
            .actuator
            .required_capabilities()
            .contains(&BackendCapability::HoverTrimUncertainty)
    );
}

#[test]
fn each_executable_uncertainty_changes_the_condition_identity() {
    let nominal = condition(42);
    let nominal_digest = nominal.canonical_digest().expect("nominal digest");
    let mut variants = Vec::new();

    let mut sensor = nominal.clone();
    sensor.sensor = SensorCondition::BoundedNoise {
        lanes: vec![SensorNoiseLane::DifferentialPressure {
            peak_amplitude_hpa: 0.5,
            update_interval_samples: 10,
        }],
    };
    variants.push(sensor);
    let mut authority = nominal.clone();
    authority.actuator.authority_scale_basis_points = 8_000;
    variants.push(authority);
    let mut hold = nominal.clone();
    hold.actuator.command_loss = CommandLossPolicy::SeededZeroOrderHold {
        fraction_basis_points: 100,
        decision_interval_samples: 100,
    };
    variants.push(hold);
    let mut hover = nominal;
    hover.controller_initialization.hover_thrust_force =
        HoverThrustForceInitialization::ScaleBaseline {
            scale_basis_points: 9_000,
        };
    variants.push(hover);

    for value in variants {
        assert_ne!(
            value.canonical_digest().expect("variant digest"),
            nominal_digest
        );
    }
}

#[test]
fn a_changed_condition_byte_changes_the_hold_schedule_identity() {
    let mut value = non_nominal();
    let first = value
        .command_hold_interval_identity(7, 0, 0, 1_001)
        .expect("first identity")
        .digest();

    value.revision = value.revision.wrapping_add(1);
    assert_ne!(
        value
            .command_hold_interval_identity(7, 0, 0, 1_001)
            .expect("changed identity")
            .digest(),
        first
    );
}

#[test]
fn the_same_seed_produces_the_same_applied_schedule_bytes() {
    let value = non_nominal();
    let first = value
        .command_hold_decisions_for_interval(7, 3, 2, 1_001)
        .expect("first schedule");
    let repeated = value
        .command_hold_decisions_for_interval(7, 3, 2, 1_001)
        .expect("repeated schedule");
    let other_seed = value
        .command_hold_decisions_for_interval(8, 3, 2, 1_001)
        .expect("other seed schedule");

    let sensor_first = value
        .sensor_references_for_sample(7, 41)
        .expect("first sensor references");
    let sensor_repeated = value
        .sensor_references_for_sample(7, 41)
        .expect("repeated sensor references");
    let sensor_other = value
        .sensor_references_for_sample(8, 41)
        .expect("other seed sensor references");

    assert_eq!(first, repeated);
    assert_ne!(first, other_seed);
    assert_eq!(first.iter().filter(|hold| **hold).count(), 1);
    assert_eq!(sensor_first, sensor_repeated);
    assert_ne!(sensor_first, sensor_other);
    assert_eq!(sensor_first.len(), 1);
}

#[test]
fn a_nominal_condition_derives_no_perturbation_stream() {
    let value = condition(42);

    assert!(
        value
            .command_hold_decisions_for_interval(7, 0, 0, 1)
            .expect("nominal schedule")
            .is_empty()
    );
    assert!(
        value
            .sensor_references_for_sample(7, 41)
            .expect("nominal references")
            .is_empty()
    );
}

/// The golden document carries every schema-4 block, the plant block
/// included, so this digest is the identity of a complete condition at the
/// current schema. A block added, removed, or reordered moves it.
#[test]
fn condition_v4_canonical_bytes_and_digest_match_the_golden_file() {
    let fixture = include_bytes!("../../../fixtures/condition-v4.golden.json");
    let fixture = fixture.strip_suffix(b"\n").unwrap_or(fixture);
    let value = ConditionSet::from_json(fixture).expect("condition v4 golden");

    assert_eq!(value.to_canonical_json().expect("canonical JSON"), fixture);
    assert_eq!(
        value
            .canonical_digest()
            .expect("canonical digest")
            .to_string(),
        "6ad7bee8a76cd3272544ca6e4d8b383023dd4ba1fb3e4438994f1d81db521453"
    );
    assert_eq!(value.schema_version, CONDITION_SET_SCHEMA_VERSION);
    assert_eq!(value.required_capabilities().len(), 4);
}

/// The hold schedule is seeded by the canonical condition digest, so this
/// position is a second, independent statement of that digest: a schedule
/// that still lands here proves the document reaching the derivation is the
/// document the fixture holds.
#[test]
fn the_golden_condition_holds_the_golden_command_hold_schedule() {
    let fixture = include_bytes!("../../../fixtures/condition-v4.golden.json");
    let fixture = fixture.strip_suffix(b"\n").unwrap_or(fixture);
    let value = ConditionSet::from_json(fixture).expect("condition v4 golden");
    let held = value
        .command_hold_decisions_for_interval(0x1112_1314_1516_1718, 0, 0, 1_001)
        .expect("golden schedule")
        .iter()
        .enumerate()
        .filter_map(|(index, hold)| hold.then_some(index))
        .collect::<Vec<_>>();

    assert_eq!(held, vec![37]);
}
