use flight_tune::{JournalEntry, JournalEvent};

use crate::test_rig::{FakeHandle, TerminalExternalAction, TestDirectory};

use super::{ADAPTER_OPERATIONS_FOR_TEST, TerminalActions, open_one};

const EXTERNAL_ACTIONS: [TerminalExternalAction; 11] = [
    TerminalExternalAction::PlanRead,
    TerminalExternalAction::Bind,
    TerminalExternalAction::SimulatorStop,
    TerminalExternalAction::ControlStop,
    TerminalExternalAction::TraceStop,
    TerminalExternalAction::ChildHealth,
    TerminalExternalAction::TraceShutdown,
    TerminalExternalAction::ChildTerminate,
    TerminalExternalAction::CausalRead,
    TerminalExternalAction::ReceiptRecover,
    TerminalExternalAction::ReceiptSeal,
];

#[test]
fn head_poison_before_each_terminal_authority_check_stops_later_external_actions() {
    for action in EXTERNAL_ACTIONS {
        assert_head_poison_boundary(action);
    }
}

fn assert_head_poison_boundary(action: TerminalExternalAction) {
    let directory = TestDirectory::new(&format!("terminal-head-poison-{action:?}"));
    let state = FakeHandle::new();
    let mut tuner = open_one(directory.path(), state.clone()).expect("open HEAD poison campaign");
    state
        .0
        .borrow_mut()
        .terminal
        .poison_head_after(action, directory.path());

    assert!(
        tuner.run_training_attempts_blocking(0).is_err(),
        "HEAD poison after {action:?} must stop the run"
    );

    let after_failure = ExternalActionSnapshot::capture(&state);
    assert_exact_terminal_prefix(action, &after_failure);
    assert_no_closure(tuner.journal().entries());
    if action == TerminalExternalAction::PlanRead {
        assert!(
            tuner
                .journal()
                .entries()
                .iter()
                .all(|entry| !matches!(entry.event, JournalEvent::RunBound { .. }))
        );
    }
    let entry_count = tuner.journal().entries().len();
    assert!(tuner.run_training_attempts_blocking(0).is_err());
    assert_eq!(ExternalActionSnapshot::capture(&state), after_failure);
    assert_eq!(tuner.journal().entries().len(), entry_count);
}

#[derive(Debug, Clone, PartialEq)]
struct ExternalActionSnapshot {
    terminal: TerminalActions,
    plan_reads: usize,
    backend: [usize; 7],
    vehicle: [usize; 3],
    evaluators: [usize; 9],
    transition_authorizations: usize,
    lifecycle: Vec<String>,
}

impl ExternalActionSnapshot {
    fn capture(handle: &FakeHandle) -> Self {
        let terminal = TerminalActions::capture(handle);
        let state = handle.0.borrow();
        Self {
            terminal,
            plan_reads: state.terminal.capabilities_read_count(),
            backend: [
                state.open_session_count,
                state.prepare_count,
                state.start_count,
                state.sample_count,
                state.sample_poll_count,
                state.stop_count,
                state.cleanup_count,
            ],
            vehicle: [
                state.vehicle.bind_count,
                state.vehicle.ensure_count,
                state.vehicle.apply_count,
            ],
            evaluators: [
                state.metric_observe_count,
                state.gate_begin_count,
                state.gate_evaluate_count,
                state.gate_finish_count,
                state.gate_cancel_count,
                state.metric_begin_count,
                state.metric_finish_count,
                state.metric_cancel_count,
                state.scenario_runs.len(),
            ],
            transition_authorizations: state.transition.authorization_count,
            lifecycle: state.lifecycle.clone(),
        }
    }
}

fn assert_exact_terminal_prefix(action: TerminalExternalAction, snapshot: &ExternalActionSnapshot) {
    let expected_operation_count = expected_operation_count(action);
    assert_eq!(snapshot.plan_reads, 1, "{action:?}");
    assert_eq!(
        snapshot.terminal.bind,
        usize::from(action != TerminalExternalAction::PlanRead),
        "{action:?}"
    );
    assert_eq!(
        snapshot.terminal.simulator_stop,
        usize::from(!matches!(
            action,
            TerminalExternalAction::PlanRead | TerminalExternalAction::Bind
        )),
        "{action:?}"
    );
    assert_eq!(
        snapshot.terminal.operations,
        ADAPTER_OPERATIONS_FOR_TEST[..expected_operation_count],
        "{action:?}"
    );
    assert_eq!(snapshot.terminal.causal_read, expected_causal(action));
    assert_eq!(snapshot.terminal.recover, expected_recover(action));
    assert_eq!(snapshot.terminal.seal, expected_seal(action));
    assert_eq!(snapshot.backend[6], 0, "{action:?}");
}

const fn expected_operation_count(action: TerminalExternalAction) -> usize {
    match action {
        TerminalExternalAction::PlanRead
        | TerminalExternalAction::Bind
        | TerminalExternalAction::SimulatorStop => 0,
        TerminalExternalAction::ControlStop => 1,
        TerminalExternalAction::TraceStop => 2,
        TerminalExternalAction::ChildHealth => 3,
        TerminalExternalAction::TraceShutdown => 4,
        TerminalExternalAction::ChildTerminate
        | TerminalExternalAction::CausalRead
        | TerminalExternalAction::ReceiptRecover
        | TerminalExternalAction::ReceiptSeal => 5,
    }
}

const fn expected_causal(action: TerminalExternalAction) -> usize {
    match action {
        TerminalExternalAction::CausalRead
        | TerminalExternalAction::ReceiptRecover
        | TerminalExternalAction::ReceiptSeal => 1,
        _ => 0,
    }
}

const fn expected_recover(action: TerminalExternalAction) -> usize {
    match action {
        TerminalExternalAction::ReceiptRecover | TerminalExternalAction::ReceiptSeal => 1,
        _ => 0,
    }
}

const fn expected_seal(action: TerminalExternalAction) -> usize {
    match action {
        TerminalExternalAction::ReceiptSeal => 1,
        _ => 0,
    }
}

fn assert_no_closure(entries: &[JournalEntry]) {
    assert!(entries.iter().all(|entry| !matches!(
        entry.event,
        JournalEvent::RunCommitted { .. }
            | JournalEvent::AttemptCompleted { .. }
            | JournalEvent::AttemptQuarantined { .. }
            | JournalEvent::CleanupRecorded { .. }
    )));
}
