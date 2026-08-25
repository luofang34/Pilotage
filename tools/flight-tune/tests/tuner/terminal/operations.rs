use flight_tune::{
    Digest, JournalEvent, RunTerminalCapabilities, RunTerminalDisposition, RunTerminalOperation,
    RunTerminalOperationStatus, RunTerminalQuarantine, RunTerminalSemanticOutcome,
    SimulatorBackend,
};

use crate::test_rig::{FakeBackend, FakeHandle, TestDirectory};

use super::{ADAPTER_OPERATIONS_FOR_TEST, latest_receipt, open_one, open_with_gate};

#[test]
fn scenario_complete_with_simulator_stop_failure_runs_the_remaining_plan() {
    assert_simulator_stop_failure("complete", 2.0, ExpectedSemantic::ScenarioComplete);
}

#[test]
fn hard_gate_abort_with_simulator_stop_failure_runs_the_remaining_plan() {
    assert_simulator_stop_failure("hard-gate", -1.0, ExpectedSemantic::HardGateAbort);
}

#[test]
fn each_adapter_operation_failure_is_quarantined_and_later_operations_run() {
    for failed in ADAPTER_OPERATIONS_FOR_TEST {
        let directory = TestDirectory::new(&format!("terminal-operation-{failed:?}"));
        let state = FakeHandle::new();
        state.0.borrow_mut().terminal.failed_operations = vec![failed];
        let mut tuner = open_one(directory.path(), state.clone()).expect("open terminal run");

        assert!(tuner.run_training_attempts_blocking(0).is_err());

        let receipt = latest_receipt(tuner.journal().entries());
        assert_terminal_failures(receipt, &[failed]);
        assert_eq!(state.0.borrow().stop_count, 1);
        assert_eq!(
            state.0.borrow().terminal.operation_order(),
            &ADAPTER_OPERATIONS_FOR_TEST
        );
    }
}

#[test]
fn multiple_adapter_failures_keep_the_complete_operation_order() {
    let directory = TestDirectory::new("terminal-operation-multiple");
    let state = FakeHandle::new();
    let failures = [
        RunTerminalOperation::ControlStop,
        RunTerminalOperation::ChildHealth,
    ];
    state.0.borrow_mut().terminal.failed_operations = failures.to_vec();
    let mut tuner = open_one(directory.path(), state.clone()).expect("open terminal run");

    assert!(tuner.run_training_attempts_blocking(0).is_err());

    let receipt = latest_receipt(tuner.journal().entries());
    assert_terminal_failures(receipt, &failures);
    assert_eq!(
        state.0.borrow().terminal.operation_order(),
        &ADAPTER_OPERATIONS_FOR_TEST
    );
    assert_eq!(
        state
            .0
            .borrow()
            .terminal
            .operation_count(RunTerminalOperation::ChildTerminate),
        1
    );
}

#[test]
fn fixed_capabilities_predeclare_not_required_operations() {
    let directory = TestDirectory::new("terminal-capability-map");
    let state = FakeHandle::new();
    state.0.borrow_mut().terminal.capabilities = RunTerminalCapabilities::new(false, true, false);
    let mut tuner = open_one(directory.path(), state.clone()).expect("open terminal run");

    assert!(tuner.run_training_attempts_blocking(0).is_err());

    assert_eq!(
        state.0.borrow().terminal.operation_order(),
        &[
            RunTerminalOperation::TraceStop,
            RunTerminalOperation::TraceShutdown,
        ]
    );
    let receipt = latest_receipt(tuner.journal().entries());
    assert!(receipt.is_completed());
    let operations = receipt.report().operations();
    for outcome in operations {
        let required = matches!(
            outcome.operation(),
            RunTerminalOperation::SimulatorStop
                | RunTerminalOperation::TraceStop
                | RunTerminalOperation::TraceShutdown
        );
        assert_eq!(
            matches!(outcome.status(), RunTerminalOperationStatus::NotRequired),
            !required,
        );
    }
}

#[test]
fn zero_child_termination_proof_becomes_a_terminal_failure() {
    let directory = TestDirectory::new("terminal-zero-child-proof");
    let state = FakeHandle::new();
    state.0.borrow_mut().terminal.child_terminate_proof = Digest::from_bytes([0; 32]);
    let mut tuner = open_one(directory.path(), state).expect("open terminal run");

    assert!(tuner.run_training_attempts_blocking(0).is_err());

    assert_terminal_failures(
        latest_receipt(tuner.journal().entries()),
        &[RunTerminalOperation::ChildTerminate],
    );
}

#[test]
fn reference_simulator_stop_is_safe_to_repeat() {
    let state = FakeHandle::new();
    let mut backend = FakeBackend::new(state.clone());

    backend.stop_blocking().expect("first simulator stop");
    backend.stop_blocking().expect("repeated simulator stop");

    assert_eq!(state.0.borrow().stop_count, 2);
}

#[derive(Clone, Copy)]
enum ExpectedSemantic {
    ScenarioComplete,
    HardGateAbort,
}

fn assert_simulator_stop_failure(label: &str, gate_limit: f64, semantic: ExpectedSemantic) {
    let directory = TestDirectory::new(&format!("terminal-stop-failure-{label}"));
    let state = FakeHandle::new();
    state.0.borrow_mut().terminal.fail_simulator_stop = true;
    let mut tuner = open_with_gate(directory.path(), state.clone(), gate_limit)
        .expect("open simulator stop run");

    assert!(tuner.run_training_attempts_blocking(0).is_err());

    let receipt = latest_receipt(tuner.journal().entries());
    assert_semantic(receipt.intent().outcome(), semantic);
    assert_terminal_failures(receipt, &[RunTerminalOperation::SimulatorStop]);
    assert_eq!(state.0.borrow().stop_count, 1);
    assert_eq!(
        state.0.borrow().terminal.operation_order(),
        &ADAPTER_OPERATIONS_FOR_TEST
    );
    assert!(
        tuner
            .journal()
            .entries()
            .iter()
            .any(|entry| matches!(entry.event, JournalEvent::AttemptQuarantined { .. }))
    );
    assert!(
        tuner
            .journal()
            .entries()
            .iter()
            .all(|entry| !matches!(entry.event, JournalEvent::AttemptCompleted { .. }))
    );
}

fn assert_semantic(actual: &RunTerminalSemanticOutcome, expected: ExpectedSemantic) {
    assert!(matches!(
        (actual, expected),
        (
            RunTerminalSemanticOutcome::ScenarioComplete { .. },
            ExpectedSemantic::ScenarioComplete
        ) | (
            RunTerminalSemanticOutcome::HardGateAbort { .. },
            ExpectedSemantic::HardGateAbort
        )
    ));
}

fn assert_terminal_failures(
    receipt: &flight_tune::RunTerminalReceipt,
    expected: &[RunTerminalOperation],
) {
    assert_eq!(
        receipt.class().disposition(),
        RunTerminalDisposition::Quarantine {
            quarantine: RunTerminalQuarantine::TerminalFailure,
        }
    );
    let failed = receipt
        .report()
        .operations()
        .iter()
        .filter_map(|outcome| {
            matches!(outcome.status(), RunTerminalOperationStatus::Failed { .. })
                .then_some(outcome.operation())
        })
        .collect::<Vec<_>>();
    assert_eq!(failed, expected);
}
