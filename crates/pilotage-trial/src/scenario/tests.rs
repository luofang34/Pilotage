#![allow(clippy::expect_used)]

use super::*;
use crate::MAX_ACTUATOR_VALUES;

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
fn a_control_selector_names_the_tagged_value_field() {
    let scenario = condition_scenario(PhaseCondition::Signal {
        selector: SignalSelector::TypedIntent {
            field: ControlValueField::VelocityYawRate,
        },
        comparison: Comparison::LessOrEqual,
        value: 0.2,
    });

    let json = scenario.to_canonical_json().expect("scenario JSON");
    let decoded = Scenario::from_json(&json).expect("scenario parse");
    assert_eq!(decoded, scenario);
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
            required_capabilities: vec![BackendCapability::SimulatorTime],
            entry_conditions: vec![PhaseCondition::Always],
            action: PhaseAction::Stimulus {
                channel: ControlChannel::Roll,
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
