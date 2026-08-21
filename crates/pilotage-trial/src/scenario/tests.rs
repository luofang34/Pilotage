#![allow(clippy::expect_used)]

use super::*;

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
