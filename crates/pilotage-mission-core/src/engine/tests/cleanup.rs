use crate::{
    DirectivePurpose, DirectiveReceipt, FlightAction, MissionAction, MissionCondition,
    MissionObservation, MissionTerminal, ReceiptResult, VehicleCondition,
};

use super::support::{document, engine, phase, succeeded, terminal, tick};

#[test]
fn abort_cleanup_reverses_phases_attempts_all_steps_and_aggregates_failures() {
    let mut first = phase("first");
    first.cleanup_actions = vec![MissionAction::Flight(FlightAction::Land {})];
    first.abort_conditions = vec![crash_condition()];
    first
        .required_capabilities
        .push(crate::MissionCapability::FlightControl);
    let mut second = phase("second");
    second.cleanup_actions = vec![
        MissionAction::Flight(FlightAction::MaintainTarget {}),
        MissionAction::Flight(FlightAction::Disarm {}),
    ];
    second.abort_conditions = vec![crash_condition()];
    second
        .required_capabilities
        .push(crate::MissionCapability::FlightControl);
    let mut engine = engine(document(vec![first, second]));

    let first_action = tick(&mut engine, 0, 0, MissionObservation::default(), Vec::new());
    let second_action = tick(
        &mut engine,
        1,
        1,
        MissionObservation::default(),
        vec![succeeded(&first_action)],
    );
    assert_eq!(second_action.directives[0].context().phase_id, "second");

    let first_cleanup = tick(&mut engine, 2, 2, crashed_observation(), Vec::new());
    assert_cleanup(&first_cleanup, 1, "second", 0, "flight.maintain_target");
    let second_cleanup = tick(
        &mut engine,
        3,
        3,
        MissionObservation::default(),
        vec![receipt(
            &first_cleanup,
            ReceiptResult::Failed {
                detail: "maintain target failed".to_owned(),
            },
        )],
    );
    assert_cleanup(&second_cleanup, 1, "second", 1, "flight.disarm");
    let third_cleanup = tick(
        &mut engine,
        4,
        4,
        MissionObservation::default(),
        vec![receipt(&second_cleanup, ReceiptResult::Succeeded {})],
    );
    assert_cleanup(&third_cleanup, 0, "first", 0, "flight.land");
    let finished = tick(
        &mut engine,
        5,
        5,
        MissionObservation::default(),
        vec![receipt(
            &third_cleanup,
            ReceiptResult::Refused {
                detail: "land refused".to_owned(),
            },
        )],
    );
    assert!(matches!(
        terminal(&finished),
        MissionTerminal::Aborted {
            cleanup_failures,
            ..
        } if cleanup_failures.len() == 2
            && cleanup_failures[0].action == "flight.maintain_target"
            && cleanup_failures[1].action == "flight.land"
    ));
}

fn crash_condition() -> MissionCondition {
    MissionCondition::Vehicle(VehicleCondition::Crashed { expected: true })
}

fn crashed_observation() -> MissionObservation {
    MissionObservation {
        vehicle: crate::VehicleObservation {
            crashed: Some(true),
            ..crate::VehicleObservation::default()
        },
        ..MissionObservation::default()
    }
}

fn receipt(output: &crate::TickOutput, result: ReceiptResult) -> DirectiveReceipt {
    DirectiveReceipt {
        action_id: output.directives[0].context().action_id,
        result,
    }
}

fn assert_cleanup(
    output: &crate::TickOutput,
    phase_index: usize,
    phase_id: &str,
    cleanup_index: usize,
    action_name: &str,
) {
    assert_eq!(output.directives.len(), 1);
    let directive = &output.directives[0];
    assert_eq!(directive.context().phase_index, phase_index);
    assert_eq!(directive.context().phase_id, phase_id);
    assert_eq!(directive.action_name(), action_name);
    assert!(matches!(
        directive.context().purpose,
        DirectivePurpose::Cleanup {
            cleanup_index: actual
        } if actual == cleanup_index
    ));
}
