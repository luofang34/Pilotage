#![allow(clippy::expect_used)]

use super::*;

fn declaration(mode: HoverEstimatorMode) -> BackendCapabilities {
    BackendCapabilities {
        schema_version: BACKEND_CAPABILITIES_SCHEMA_VERSION,
        backend: ArtifactIdentity {
            id: "typed-backend".to_owned(),
            revision: "r1".to_owned(),
            digest: Digest::from_bytes([9; 32]),
        },
        capabilities: vec![
            BackendCapability::SensorPerturbation,
            BackendCapability::ActuatorAuthority,
            BackendCapability::CommandHold,
            BackendCapability::HoverTrimUncertainty,
        ],
        hover_estimator_mode: mode,
    }
}

#[test]
fn every_capability_has_one_stable_snake_case_name() {
    let capabilities = [
        BackendCapability::Reset,
        BackendCapability::LifecycleState,
        BackendCapability::SimulatorTime,
        BackendCapability::ConditionControl,
        BackendCapability::KinematicTruth,
        BackendCapability::DeterministicSeed,
        BackendCapability::ArmDisarm,
        BackendCapability::ContactState,
        BackendCapability::WindControl,
        BackendCapability::TurbulenceControl,
        BackendCapability::OperatorVelocityControl,
        BackendCapability::DirectAttitudeThrustControl,
        BackendCapability::SensorPerturbation,
        BackendCapability::ActuatorAuthority,
        BackendCapability::CommandHold,
        BackendCapability::HoverTrimUncertainty,
    ];
    for capability in capabilities {
        let encoded = serde_json::to_string(&capability).expect("capability JSON");
        assert_eq!(encoded, format!("\"{}\"", capability.as_str()));
    }
    let names = capabilities
        .iter()
        .map(|capability| capability.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(names.len(), capabilities.len());
}

#[test]
fn estimator_mode_is_mutually_exclusive_and_content_identified() {
    let online = declaration(HoverEstimatorMode::Online);
    let disabled = declaration(HoverEstimatorMode::Disabled);
    let frozen = declaration(HoverEstimatorMode::Frozen);

    assert_eq!(HoverEstimatorMode::default(), HoverEstimatorMode::Online);
    assert!(!HoverEstimatorMode::Online.is_inactive());
    assert!(HoverEstimatorMode::Disabled.is_inactive());
    assert!(HoverEstimatorMode::Frozen.is_inactive());
    assert_ne!(
        online.canonical_digest().expect("online digest"),
        disabled.canonical_digest().expect("disabled digest")
    );
    assert_ne!(
        disabled.canonical_digest().expect("disabled digest"),
        frozen.canonical_digest().expect("frozen digest")
    );
}

#[test]
fn estimator_mode_json_rejects_missing_unknown_and_invalid_values() {
    let mut value =
        serde_json::to_value(declaration(HoverEstimatorMode::Frozen)).expect("capability value");
    value
        .as_object_mut()
        .expect("capability object")
        .remove("hover_estimator_mode");
    assert!(
        BackendCapabilities::from_json(&serde_json::to_vec(&value).expect("missing JSON")).is_err()
    );

    value["hover_estimator_mode"] = serde_json::Value::String("paused".to_owned());
    assert!(
        BackendCapabilities::from_json(&serde_json::to_vec(&value).expect("invalid JSON")).is_err()
    );

    value["hover_estimator_mode"] = serde_json::Value::String("frozen".to_owned());
    value["extra"] = serde_json::Value::Bool(true);
    assert!(
        BackendCapabilities::from_json(&serde_json::to_vec(&value).expect("unknown JSON")).is_err()
    );
}

#[test]
fn backend_capabilities_match_canonical_golden_bytes() {
    let fixture = include_bytes!("../../fixtures/backend-capabilities-v2.golden.json");
    let fixture = fixture.strip_suffix(b"\n").unwrap_or(fixture);
    let value = BackendCapabilities::from_json(fixture).expect("backend capabilities golden");

    assert_eq!(value.to_canonical_json().expect("canonical JSON"), fixture);
    assert_eq!(
        value
            .canonical_digest()
            .expect("canonical digest")
            .to_string(),
        "8b83e53b4baee6e34da46d3a6faf3e077f16fb3ddd24e287589f8889a0e6ce52"
    );
    assert_eq!(value.schema_version, BACKEND_CAPABILITIES_SCHEMA_VERSION);
    assert_eq!(value.hover_estimator_mode, HoverEstimatorMode::Frozen);
}
