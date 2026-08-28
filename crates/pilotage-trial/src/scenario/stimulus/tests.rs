#![allow(clippy::expect_used, clippy::panic)]

use super::*;
use crate::{
    BackendCapability, Comparison, Phase, PhaseAction, PhaseCondition, SCENARIO_SCHEMA_VERSION,
    Scenario, Waveform,
};

fn operator_envelope(channel: ControlChannel) -> StimulusEnvelope {
    let (unit, endpoint) = match channel {
        ControlChannel::Yaw => (PhysicalUnit::RadiansPerSecond, 1.5),
        _ => (PhysicalUnit::MetersPerSecond, 5.0),
    };
    StimulusEnvelope {
        id: "alia250.operator".to_owned(),
        revision: 1,
        unit,
        reference: ReferenceRule::Zero,
        negative_endpoint: -endpoint,
        neutral: 0.0,
        positive_endpoint: endpoint,
    }
}

fn direct_envelope(channel: ControlChannel) -> StimulusEnvelope {
    match channel {
        ControlChannel::Vertical => StimulusEnvelope {
            id: "alia250.direct.collective".to_owned(),
            revision: 1,
            unit: PhysicalUnit::NormalizedCollectiveForce,
            reference: ReferenceRule::IdentifiedHoverTrim,
            negative_endpoint: -0.3,
            neutral: 0.0,
            positive_endpoint: 0.3,
        },
        _ => StimulusEnvelope {
            id: "alia250.direct.attitude".to_owned(),
            revision: 1,
            unit: PhysicalUnit::Radians,
            reference: ReferenceRule::EffectiveSetpointAtEntry,
            negative_endpoint: -0.25,
            neutral: 0.0,
            positive_endpoint: 0.25,
        },
    }
}

fn envelope_for(family: ControlFamily, channel: ControlChannel) -> StimulusEnvelope {
    match family {
        ControlFamily::OperatorVelocity => operator_envelope(channel),
        ControlFamily::DirectAttitudeThrust => direct_envelope(channel),
    }
}

fn stimulus_scenario(
    family: ControlFamily,
    channel: ControlChannel,
    mapping: StimulusMapping,
    envelope: StimulusEnvelope,
) -> Scenario {
    Scenario {
        schema_version: SCENARIO_SCHEMA_VERSION,
        id: "stimulus-identity".to_owned(),
        revision: 1,
        phases: vec![Phase {
            id: "stimulus".to_owned(),
            max_sim_time_ns: 2_000_000_000,
            required_capabilities: vec![BackendCapability::SimulatorTime, family.capability()],
            entry_conditions: vec![PhaseCondition::Always],
            action: PhaseAction::Stimulus {
                family,
                channel,
                mapping,
                envelope,
                waveform: Waveform::Pulse {
                    value: 0.5,
                    duration_ns: 500_000_000,
                },
            },
            exit_conditions: vec![PhaseCondition::SimulatorTime {
                comparison: Comparison::GreaterOrEqual,
                value_ns: 1_000_000_000,
            }],
            abort_conditions: Vec::new(),
        }],
    }
}

fn valid_scenario(family: ControlFamily, channel: ControlChannel) -> Scenario {
    stimulus_scenario(
        family,
        channel,
        family.mapping(),
        envelope_for(family, channel),
    )
}

const FAMILIES: [ControlFamily; 2] = [
    ControlFamily::OperatorVelocity,
    ControlFamily::DirectAttitudeThrust,
];

const CHANNELS: [ControlChannel; 4] = [
    ControlChannel::Roll,
    ControlChannel::Pitch,
    ControlChannel::Vertical,
    ControlChannel::Yaw,
];

#[test]
fn each_family_and_channel_has_a_canonical_round_trip() {
    for family in FAMILIES {
        for channel in CHANNELS {
            let scenario = valid_scenario(family, channel);
            let json = scenario
                .to_canonical_json()
                .unwrap_or_else(|error| panic!("{family:?} {channel:?} encodes: {error}"));
            let decoded = Scenario::from_json(&json)
                .unwrap_or_else(|error| panic!("{family:?} {channel:?} decodes: {error}"));
            assert_eq!(decoded, scenario, "{family:?} {channel:?} round trip");
        }
    }
}

#[test]
fn a_changed_family_changes_the_scenario_digest() {
    let operator = valid_scenario(ControlFamily::OperatorVelocity, ControlChannel::Roll);
    let direct = valid_scenario(ControlFamily::DirectAttitudeThrust, ControlChannel::Roll);

    assert_ne!(
        operator.canonical_digest().expect("operator digest"),
        direct.canonical_digest().expect("direct digest")
    );
}

#[test]
fn a_changed_envelope_changes_both_digests() {
    let scenario = valid_scenario(ControlFamily::OperatorVelocity, ControlChannel::Roll);
    let base_scenario_digest = scenario.canonical_digest().expect("base scenario digest");
    let base_envelope = operator_envelope(ControlChannel::Roll);
    let base_envelope_digest = base_envelope
        .canonical_digest()
        .expect("base envelope digest");

    let mut wider = base_envelope.clone();
    wider.positive_endpoint = 6.0;
    let mut revised = base_envelope.clone();
    revised.revision = 2;

    for changed in [wider, revised] {
        assert_ne!(
            base_envelope_digest,
            changed.canonical_digest().expect("changed envelope digest")
        );
        let changed_scenario = stimulus_scenario(
            ControlFamily::OperatorVelocity,
            ControlChannel::Roll,
            StimulusMapping::CandidateBoundCurve,
            changed,
        );
        assert_ne!(
            base_scenario_digest,
            changed_scenario
                .canonical_digest()
                .expect("changed scenario digest")
        );
    }
}

#[test]
fn a_unit_substitution_fails_validation() {
    let mut envelope = operator_envelope(ControlChannel::Roll);
    envelope.unit = PhysicalUnit::RadiansPerSecond;
    let scenario = stimulus_scenario(
        ControlFamily::OperatorVelocity,
        ControlChannel::Roll,
        StimulusMapping::CandidateBoundCurve,
        envelope,
    );

    assert!(matches!(
        scenario.validate(),
        Err(ValidationError::InvalidStimulus {
            source: StimulusError::UnitMismatch {
                expected: "meters_per_second",
                actual: "radians_per_second",
                ..
            },
            ..
        })
    ));
}

#[test]
fn a_reference_substitution_fails_validation() {
    let mut envelope = direct_envelope(ControlChannel::Vertical);
    envelope.reference = ReferenceRule::Zero;
    let scenario = stimulus_scenario(
        ControlFamily::DirectAttitudeThrust,
        ControlChannel::Vertical,
        StimulusMapping::AffineExact,
        envelope,
    );

    assert!(matches!(
        scenario.validate(),
        Err(ValidationError::InvalidStimulus {
            source: StimulusError::ReferenceMismatch {
                expected: "identified_hover_trim",
                actual: "zero",
                ..
            },
            ..
        })
    ));
}

#[test]
fn a_family_and_mapping_mismatch_fails_validation() {
    let scenario = stimulus_scenario(
        ControlFamily::OperatorVelocity,
        ControlChannel::Roll,
        StimulusMapping::AffineExact,
        operator_envelope(ControlChannel::Roll),
    );

    assert!(matches!(
        scenario.validate(),
        Err(ValidationError::InvalidStimulus {
            source: StimulusError::MappingMismatch {
                expected: "candidate_bound_curve",
                actual: "affine_exact",
                ..
            },
            ..
        })
    ));
}

#[test]
fn an_invalid_envelope_shape_fails_validation() {
    let base = direct_envelope(ControlChannel::Roll);
    let mut reversed = base.clone();
    reversed.negative_endpoint = 0.5;
    let mut zero_span = base.clone();
    zero_span.negative_endpoint = 0.0;
    zero_span.positive_endpoint = 0.0;
    let mut outside = base.clone();
    outside.neutral = 0.25;
    let mut infinite = base;
    infinite.positive_endpoint = f64::INFINITY;

    assert!(matches!(
        reversed.validate_values(),
        Err(StimulusError::ReversedEndpoints { .. })
    ));
    assert!(matches!(
        zero_span.validate_values(),
        Err(StimulusError::ZeroSpan { .. })
    ));
    assert!(matches!(
        outside.validate_values(),
        Err(StimulusError::NeutralOutsideEndpoints { .. })
    ));
    assert!(matches!(
        infinite.validate_values(),
        Err(StimulusError::NonFiniteValue {
            name: "positive_endpoint"
        })
    ));
}

#[test]
fn an_exact_mapping_resolves_two_affine_segments() {
    let envelope = StimulusEnvelope {
        id: "asymmetric.collective".to_owned(),
        revision: 1,
        unit: PhysicalUnit::NormalizedCollectiveForce,
        reference: ReferenceRule::IdentifiedHoverTrim,
        negative_endpoint: -0.2,
        neutral: 0.1,
        positive_endpoint: 0.5,
    };

    let mapping = StimulusMapping::AffineExact;
    assert!((mapping.resolve_exact(&envelope, -1.0).expect("minimum") + 0.2).abs() < 1.0e-12);
    assert!((mapping.resolve_exact(&envelope, 0.0).expect("neutral") - 0.1).abs() < 1.0e-12);
    assert!((mapping.resolve_exact(&envelope, 1.0).expect("maximum") - 0.5).abs() < 1.0e-12);
    assert!((mapping.resolve_exact(&envelope, -0.5).expect("half low") + 0.05).abs() < 1.0e-12);
    assert!((mapping.resolve_exact(&envelope, 0.5).expect("half high") - 0.3).abs() < 1.0e-12);
    assert!(matches!(
        mapping.resolve_exact(&envelope, 1.5),
        Err(StimulusError::NormalizedOutOfRange { .. })
    ));
}

#[test]
fn a_candidate_bound_mapping_resolves_no_exact_value() {
    let envelope = operator_envelope(ControlChannel::Roll);

    assert!(matches!(
        StimulusMapping::CandidateBoundCurve.resolve_exact(&envelope, 0.5),
        Err(StimulusError::InexactMapping {
            mapping: "candidate_bound_curve"
        })
    ));
}

#[test]
fn a_stimulus_phase_must_declare_its_family_capability() {
    let mut scenario = valid_scenario(ControlFamily::DirectAttitudeThrust, ControlChannel::Yaw);
    scenario.phases[0].required_capabilities = vec![
        BackendCapability::SimulatorTime,
        BackendCapability::OperatorVelocityControl,
    ];

    assert!(matches!(
        scenario.validate(),
        Err(ValidationError::UnsupportedCapability { .. })
    ));
}

#[test]
fn a_document_without_a_family_fails_decode() {
    let scenario = valid_scenario(ControlFamily::OperatorVelocity, ControlChannel::Pitch);
    let json = scenario.to_canonical_json().expect("scenario JSON");
    let mut document: serde_json::Value = serde_json::from_slice(&json).expect("JSON value");
    let action = document["phases"][0]["action"]
        .as_object_mut()
        .expect("action object");
    assert_eq!(action["family"], "operator_velocity");
    action.remove("family");
    let without_family = serde_json::to_vec(&document).expect("missing-family JSON");

    assert!(Scenario::from_json(&without_family).is_err());
}

#[test]
fn an_envelope_rejects_an_unknown_field_and_an_unknown_enum_value() {
    let scenario = valid_scenario(ControlFamily::OperatorVelocity, ControlChannel::Vertical);
    let json = scenario.to_canonical_json().expect("scenario JSON");
    let mut extra: serde_json::Value = serde_json::from_slice(&json).expect("JSON value");
    extra["phases"][0]["action"]["envelope"]["hover_trim"] = serde_json::json!(0.5);
    let mut unknown_unit: serde_json::Value = serde_json::from_slice(&json).expect("JSON value");
    unknown_unit["phases"][0]["action"]["envelope"]["unit"] = serde_json::json!("knots");

    assert!(
        Scenario::from_json(&serde_json::to_vec(&extra).expect("extra JSON")).is_err(),
        "an unknown envelope field is refused"
    );
    assert!(
        Scenario::from_json(&serde_json::to_vec(&unknown_unit).expect("unit JSON")).is_err(),
        "an unknown unit is refused"
    );
}

#[test]
fn a_second_vehicle_uses_its_own_valid_envelope() {
    let x500 = StimulusEnvelope {
        id: "x500.operator.vertical".to_owned(),
        revision: 3,
        unit: PhysicalUnit::MetersPerSecond,
        reference: ReferenceRule::Zero,
        negative_endpoint: -2.5,
        neutral: 0.0,
        positive_endpoint: 1.5,
    };
    let alia = operator_envelope(ControlChannel::Vertical);
    let scenario = stimulus_scenario(
        ControlFamily::OperatorVelocity,
        ControlChannel::Vertical,
        StimulusMapping::CandidateBoundCurve,
        x500.clone(),
    );

    assert!(scenario.validate().is_ok());
    assert_ne!(
        x500.canonical_digest().expect("second vehicle digest"),
        alia.canonical_digest().expect("first vehicle digest")
    );
    let json = scenario.to_canonical_json().expect("scenario JSON");
    assert_eq!(
        Scenario::from_json(&json).expect("scenario decode"),
        scenario
    );
}

#[test]
fn a_waveform_value_outside_the_normalized_range_fails_validation() {
    let mut scenario = valid_scenario(ControlFamily::OperatorVelocity, ControlChannel::Roll);
    scenario.phases[0].action = PhaseAction::Stimulus {
        family: ControlFamily::OperatorVelocity,
        channel: ControlChannel::Roll,
        mapping: StimulusMapping::CandidateBoundCurve,
        envelope: operator_envelope(ControlChannel::Roll),
        waveform: Waveform::Step { value: 1.5 },
    };

    assert!(matches!(
        scenario.validate(),
        Err(ValidationError::OutOfRange { .. })
    ));
}
