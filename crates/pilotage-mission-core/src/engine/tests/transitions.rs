use crate::{
    Comparison, DeadlineClass, DirectiveReceipt, EngineState, MissionCondition, MissionObservation,
    MissionTerminal, ReceiptResult, SimulatorCondition, VehicleCondition,
};

use super::support::{
    PHASE_DEADLINE_NS, RECEIPT_TIMEOUT_NS, WALL_DEADLINE_NS, condition_phase, document, engine,
    phase, succeeded, terminal, tick,
};

#[test]
fn entry_and_completion_conditions_gate_their_transitions() {
    let phase = condition_phase(
        vec![MissionCondition::Vehicle(VehicleCondition::LinkValid {
            expected: true,
        })],
        vec![MissionCondition::Vehicle(VehicleCondition::GroundContact {
            expected: true,
        })],
        Vec::new(),
    );
    let mut engine = engine(document(vec![phase]));
    let waiting = tick(&mut engine, 0, 0, observation(false, false), Vec::new());
    assert!(waiting.directives.is_empty());
    assert!(matches!(
        waiting.state,
        EngineState::Running {
            stage: crate::PhaseStage::WaitingForEntry {},
            ..
        }
    ));
    let entered = tick(&mut engine, 1, 1, observation(true, false), Vec::new());
    assert_eq!(entered.directives.len(), 1);
    let acknowledged = tick(
        &mut engine,
        2,
        2,
        observation(true, false),
        vec![succeeded(&entered)],
    );
    assert!(matches!(
        acknowledged.state,
        EngineState::Running {
            stage: crate::PhaseStage::WaitingForCompletion {},
            ..
        }
    ));
    let complete = tick(&mut engine, 3, 3, observation(true, true), Vec::new());
    assert!(matches!(
        terminal(&complete),
        MissionTerminal::Complete { .. }
    ));
}

#[test]
fn abort_condition_gates_the_aborted_terminal() {
    let phase = condition_phase(
        Vec::new(),
        vec![MissionCondition::Vehicle(VehicleCondition::GroundContact {
            expected: true,
        })],
        vec![MissionCondition::Vehicle(VehicleCondition::Crashed {
            expected: true,
        })],
    );
    let mut engine = engine(document(vec![phase]));
    let active = tick(&mut engine, 0, 0, observation(true, false), Vec::new());
    let waiting = tick(
        &mut engine,
        1,
        1,
        observation(true, false),
        vec![succeeded(&active)],
    );
    assert!(matches!(waiting.state, EngineState::Running { .. }));
    let aborted = tick(&mut engine, 2, 2, crashed_observation(), Vec::new());
    assert!(matches!(
        terminal(&aborted),
        MissionTerminal::Aborted {
            cause: crate::AbortCause::Condition { condition_index: 0 },
            ..
        }
    ));
}

#[test]
fn refused_receipt_names_the_refusing_phase_and_action() {
    let mut engine = engine(document(vec![phase("refusing-phase")]));
    let emitted = tick(&mut engine, 0, 0, MissionObservation::default(), Vec::new());
    let refused = tick(
        &mut engine,
        1,
        1,
        MissionObservation::default(),
        vec![DirectiveReceipt {
            action_id: emitted.directives[0].context().action_id,
            result: ReceiptResult::Refused {
                detail: "unsupported action".to_owned(),
            },
        }],
    );
    assert!(matches!(
        terminal(&refused),
        MissionTerminal::Refused {
            phase_id,
            action,
            detail,
            ..
        } if phase_id == "refusing-phase"
            && action == "flight.arm"
            && detail == "unsupported action"
    ));
}

#[test]
fn phase_and_wall_deadlines_have_distinct_typed_classes() {
    let mut phase_engine = engine(document(vec![phase("phase-deadline")]));
    let phase_output = tick(
        &mut phase_engine,
        PHASE_DEADLINE_NS,
        1,
        MissionObservation::default(),
        Vec::new(),
    );
    assert!(matches!(
        terminal(&phase_output),
        MissionTerminal::DeadlineExceeded {
            deadline: DeadlineClass::PhaseSimulatorTime { .. },
            ..
        }
    ));

    let mut wall_engine = engine(document(vec![phase("wall-deadline")]));
    let wall_output = tick(
        &mut wall_engine,
        1,
        WALL_DEADLINE_NS,
        MissionObservation::default(),
        Vec::new(),
    );
    assert!(matches!(
        terminal(&wall_output),
        MissionTerminal::DeadlineExceeded {
            deadline: DeadlineClass::MissionWall { .. },
            ..
        }
    ));
}

#[test]
fn entry_wait_is_part_of_the_phase_deadline() {
    let phase = condition_phase(
        vec![MissionCondition::Simulator(SimulatorCondition::Time {
            comparison: Comparison::GreaterThan,
            value_ns: PHASE_DEADLINE_NS,
        })],
        Vec::new(),
        Vec::new(),
    );
    let mut engine = engine(document(vec![phase]));
    let waiting = tick(
        &mut engine,
        PHASE_DEADLINE_NS.wrapping_sub(1),
        1,
        MissionObservation::default(),
        Vec::new(),
    );
    assert!(waiting.directives.is_empty());
    let deadline = tick(
        &mut engine,
        PHASE_DEADLINE_NS,
        2,
        MissionObservation::default(),
        Vec::new(),
    );
    assert!(matches!(
        terminal(&deadline),
        MissionTerminal::DeadlineExceeded {
            deadline: DeadlineClass::PhaseSimulatorTime { .. },
            ..
        }
    ));
}

#[test]
fn missing_receipt_has_its_own_terminal_class() {
    let mut engine = engine(document(vec![phase("receipt-timeout")]));
    let emitted = tick(&mut engine, 0, 0, MissionObservation::default(), Vec::new());
    let action_id = emitted.directives[0].context().action_id;
    let timeout = tick(
        &mut engine,
        1,
        RECEIPT_TIMEOUT_NS,
        MissionObservation::default(),
        Vec::new(),
    );
    assert!(matches!(
        terminal(&timeout),
        MissionTerminal::ReceiptTimeout {
            action_id: timed_out,
            ..
        } if *timed_out == action_id
    ));
}

#[test]
fn retryable_receipts_stop_at_the_document_retry_limit() {
    let mut engine = engine(document(vec![phase("bounded-retry")]));
    let first = tick(&mut engine, 0, 0, MissionObservation::default(), Vec::new());
    let retry = tick(
        &mut engine,
        1,
        1,
        MissionObservation::default(),
        vec![retryable(&first, "first retry")],
    );
    let stopped = tick(
        &mut engine,
        2,
        2,
        MissionObservation::default(),
        vec![retryable(&retry, "limit reached")],
    );
    assert!(matches!(
        terminal(&stopped),
        MissionTerminal::Aborted {
            cause: crate::AbortCause::RetryLimitExceeded { retry_limit: 1, .. },
            ..
        }
    ));
}

fn retryable(output: &crate::TickOutput, detail: &str) -> DirectiveReceipt {
    DirectiveReceipt {
        action_id: output.directives[0].context().action_id,
        result: ReceiptResult::Retryable {
            detail: detail.to_owned(),
        },
    }
}

fn observation(link_valid: bool, ground_contact: bool) -> MissionObservation {
    MissionObservation {
        vehicle: crate::VehicleObservation {
            link_valid: Some(link_valid),
            ground_contact: Some(ground_contact),
            ..crate::VehicleObservation::default()
        },
        ..MissionObservation::default()
    }
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
