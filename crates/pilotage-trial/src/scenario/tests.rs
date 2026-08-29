#![allow(clippy::expect_used)]

use super::*;
use crate::{BackendCapability, ControlValue, MAX_ACTUATOR_VALUES, ReferenceFrame, Vector3};

fn start_scenario(target: StartState) -> Scenario {
    Scenario {
        schema_version: SCENARIO_SCHEMA_VERSION,
        id: "relative-start".to_owned(),
        revision: 1,
        phases: vec![Phase {
            id: "reach-start".to_owned(),
            max_sim_time_ns: 5_000_000_000,
            required_capabilities: vec![
                BackendCapability::SimulatorTime,
                BackendCapability::KinematicTruth,
            ],
            entry_conditions: vec![PhaseCondition::Always],
            action: PhaseAction::ReachStartState { target },
            exit_conditions: vec![PhaseCondition::Always],
            abort_conditions: vec![],
        }],
    }
}

#[test]
fn a_relative_start_state_has_a_bounded_heading() {
    let mut target = StartState {
        relative_position_ned_m: [0.0, 0.0, -5.0],
        heading: StartHeading::ResetOffset { radians: 0.0 },
    };
    assert!(start_scenario(target).validate().is_ok());

    target.heading = StartHeading::True { radians: 4.0 };
    assert!(matches!(
        start_scenario(target).validate(),
        Err(ValidationError::OutOfRange { .. })
    ));
}

fn condition_scenario(condition: PhaseCondition) -> Scenario {
    Scenario {
        schema_version: SCENARIO_SCHEMA_VERSION,
        id: "condition-validation".to_owned(),
        revision: 1,
        phases: vec![Phase {
            id: "observe".to_owned(),
            max_sim_time_ns: 1_000_000_000,
            required_capabilities: vec![BackendCapability::SimulatorTime],
            entry_conditions: vec![PhaseCondition::Always],
            action: PhaseAction::Observe,
            exit_conditions: vec![condition],
            abort_conditions: Vec::new(),
        }],
    }
}

#[test]
fn a_control_selector_names_the_tagged_value_field_and_frame() {
    let scenario = condition_scenario(PhaseCondition::Signal {
        selector: SignalSelector::TypedIntent {
            field: ControlValueField::VelocityYawRate {
                expected_frame: ReferenceFrame::BodyFrd,
            },
        },
        comparison: Comparison::LessOrEqual,
        value: 0.2,
    });

    let json = scenario.to_canonical_json().expect("scenario JSON");
    let decoded = Scenario::from_json(&json).expect("scenario parse");
    assert_eq!(decoded, scenario);

    let mut document: serde_json::Value = serde_json::from_slice(&json).expect("JSON value");
    let field = &mut document["phases"][0]["exit_conditions"][0]["selector"]["field"];
    assert_eq!(field["expected_frame"], "body_frd");
    field
        .as_object_mut()
        .expect("control value field")
        .remove("expected_frame");
    let missing_frame = serde_json::to_vec(&document).expect("missing-frame JSON");
    assert!(Scenario::from_json(&missing_frame).is_err());
}

#[test]
fn a_control_selector_rejects_a_different_reference_frame() {
    let selector = ControlValueField::VelocityX {
        expected_frame: ReferenceFrame::BodyFrd,
    };
    let body_value = velocity_value(ReferenceFrame::BodyFrd);
    let local_value = velocity_value(ReferenceFrame::LocalNed);

    assert_eq!(selector.select(&body_value), Ok(1.0));
    assert!(matches!(
        selector.select(&local_value),
        Err(SignalSelectionError::ReferenceFrameMismatch {
            expected: "body_frd",
            actual: "local_ned",
            ..
        })
    ));
}

#[test]
fn a_control_selector_rejects_a_different_value_variant() {
    let selector = ControlValueField::AttitudeThrust {
        expected_frame: ReferenceFrame::BodyFrd,
    };
    let value = ControlValue::BodyRateThrust {
        body_rates_rad_s: Vector3 {
            x: 0.1,
            y: 0.2,
            z: 0.3,
        },
        thrust: 0.4,
    };

    assert!(matches!(
        selector.select(&value),
        Err(SignalSelectionError::ControlValueVariantMismatch {
            expected: "attitude_thrust",
            actual: "body_rate_thrust",
            ..
        })
    ));
}

#[test]
fn a_control_selector_rejects_an_unavailable_scalar_channel() {
    let selector = ControlValueField::ScalarChannel { index: 1 };
    let value = ControlValue::ScalarChannels { values: vec![0.2] };

    assert_eq!(
        selector.select(&value),
        Err(SignalSelectionError::ScalarChannelUnavailable { index: 1, count: 1 })
    );
}

fn velocity_value(frame: ReferenceFrame) -> ControlValue {
    ControlValue::Velocity {
        frame,
        linear_mps: Vector3 {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        },
        yaw_rate_rad_s: 0.4,
    }
}

#[test]
fn a_condition_selector_uses_a_stable_name() {
    let mut scenario = condition_scenario(PhaseCondition::Signal {
        selector: SignalSelector::ConditionValue {
            name: "gust_phase".to_owned(),
        },
        comparison: Comparison::GreaterOrEqual,
        value: 1.0,
    });
    scenario.phases[0]
        .required_capabilities
        .push(BackendCapability::ConditionControl);
    assert!(scenario.validate().is_ok());

    if let PhaseCondition::Signal {
        selector: SignalSelector::ConditionValue { name },
        ..
    } = &mut scenario.phases[0].exit_conditions[0]
    {
        name.clear();
    }
    assert!(matches!(
        scenario.validate(),
        Err(ValidationError::EmptyText { .. })
    ));
}

#[test]
fn a_scalar_channel_selector_has_a_hard_index_limit() {
    let scenario = condition_scenario(PhaseCondition::Signal {
        selector: SignalSelector::AdapterDemand {
            field: ControlValueField::ScalarChannel {
                index: MAX_ACTUATOR_VALUES as u16,
            },
        },
        comparison: Comparison::LessOrEqual,
        value: 0.0,
    });

    assert!(matches!(
        scenario.validate(),
        Err(ValidationError::OutOfRange { .. })
    ));
}

#[test]
fn an_absolute_comparison_rejects_a_negative_threshold() {
    let scenario = condition_scenario(PhaseCondition::Signal {
        selector: SignalSelector::NormalizedControl {
            channel: ControlChannel::Roll,
        },
        comparison: Comparison::AbsoluteLessOrEqual,
        value: -0.1,
    });

    assert!(matches!(
        scenario.validate(),
        Err(ValidationError::OutOfRange { .. })
    ));
}

#[test]
fn a_multisine_rejects_an_excessive_composed_envelope() {
    let waveform = Waveform::Multisine {
        bias: 0.5,
        components: vec![
            SineComponent {
                amplitude: 0.4,
                frequency_hz: 1.0,
                phase_rad: 0.0,
            },
            SineComponent {
                amplitude: -0.2,
                frequency_hz: 2.0,
                phase_rad: 0.0,
            },
        ],
        duration_ns: 1_000_000_000,
    };
    let scenario = Scenario {
        schema_version: SCENARIO_SCHEMA_VERSION,
        id: "bounded-multisine".to_owned(),
        revision: 1,
        phases: vec![Phase {
            id: "stimulus".to_owned(),
            max_sim_time_ns: 1_000_000_000,
            required_capabilities: vec![
                BackendCapability::SimulatorTime,
                BackendCapability::OperatorVelocityControl,
            ],
            entry_conditions: vec![PhaseCondition::Always],
            action: PhaseAction::Stimulus {
                family: ControlFamily::OperatorVelocity,
                channel: ControlChannel::Roll,
                mapping: StimulusMapping::CandidateBoundCurve,
                envelope: StimulusEnvelope {
                    id: "bounded-multisine.roll".to_owned(),
                    revision: 1,
                    unit: PhysicalUnit::MetersPerSecond,
                    reference: ReferenceRule::Zero,
                    negative_endpoint: -4.0,
                    neutral: 0.0,
                    positive_endpoint: 4.0,
                },
                waveform,
            },
            exit_conditions: vec![PhaseCondition::Always],
            abort_conditions: Vec::new(),
        }],
    };

    assert!(matches!(
        scenario.validate(),
        Err(ValidationError::OutOfRange { .. })
    ));
}
