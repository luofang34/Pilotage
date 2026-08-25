use flight_tune::{
    JournalEvent, RunTerminalBindingStatus, RunTerminalDisposition, RunTerminalOperation,
    RunTerminalOperationStatus, RunTerminalQuarantine, RunTerminalSemanticOutcome,
};

use crate::test_rig::FakeHandle;

use super::{CrashBoundary, DurableRun, TerminalActions, durable_run, latest_receipt, open_one};

const ADAPTER_OPERATIONS: [RunTerminalOperation; 5] = [
    RunTerminalOperation::ControlStop,
    RunTerminalOperation::TraceStop,
    RunTerminalOperation::ChildHealth,
    RunTerminalOperation::TraceShutdown,
    RunTerminalOperation::ChildTerminate,
];

#[test]
fn crash_after_run_prepared_recovers_exactly_once_across_two_opens() {
    let run = durable_run("terminal-recover-prepared", Vec::new());
    assert_recovery(
        run,
        CrashBoundary::RunPrepared,
        ReceiptSeed::Absent,
        ExpectedActions::runtime_containment(),
        false,
    );
}

#[test]
fn crash_after_run_bound_recovers_exactly_once_across_two_opens() {
    let run = durable_run("terminal-recover-bound", Vec::new());
    assert_recovery(
        run,
        CrashBoundary::RunBound,
        ReceiptSeed::Absent,
        ExpectedActions::active_containment(),
        false,
    );
}

#[test]
fn run_bound_is_contained_before_recovery_writes_a_new_event() {
    let run = durable_run("terminal-recover-bound-order", Vec::new());
    run.rewind(CrashBoundary::RunBound);
    let state = FakeHandle::new();
    state.0.borrow_mut().expected_head_event_on_stop =
        Some((run.directory.path().to_path_buf(), "run_bound".to_owned()));

    let recovered = open_one(run.directory.path(), state.clone()).expect("recovery open");

    assert!(state.0.borrow().expected_head_event_on_stop.is_none());
    assert!(matches!(
        latest_receipt(recovered.journal().entries())
            .intent()
            .outcome(),
        RunTerminalSemanticOutcome::Recovery
    ));
}

#[test]
fn crash_after_terminal_intent_recovers_exactly_once_across_two_opens() {
    let run = durable_run("terminal-recover-intent", Vec::new());
    assert_recovery(
        run,
        CrashBoundary::IntentPrepared,
        ReceiptSeed::Absent,
        ExpectedActions::active_containment(),
        true,
    );
}

#[test]
fn failed_rebind_cannot_recover_a_completed_intent() {
    let run = durable_run("terminal-recover-bind-failure", Vec::new());
    run.rewind(CrashBoundary::IntentPrepared);
    let state = FakeHandle::new();
    state.0.borrow_mut().terminal.lose_bind_acknowledgement = true;

    let first = open_one(run.directory.path(), state.clone()).expect("first recovery open");
    let receipt = latest_receipt(first.journal().entries()).clone();
    assert_eq!(
        receipt.class().disposition(),
        RunTerminalDisposition::Quarantine {
            quarantine: RunTerminalQuarantine::TerminalFailure,
        }
    );
    assert!(matches!(
        receipt.report().binding_status(),
        RunTerminalBindingStatus::Failed { .. }
    ));
    assert!(receipt.report().operations().iter().all(|outcome| matches!(
        outcome.status(),
        RunTerminalOperationStatus::Succeeded { .. }
    )));
    assert_eq!(
        state.0.borrow().terminal.operation_order(),
        &ADAPTER_OPERATIONS
    );
    let first_actions = TerminalActions::capture(&state);
    drop(first);

    let second = open_one(run.directory.path(), state.clone()).expect("second recovery open");
    assert_eq!(latest_receipt(second.journal().entries()), &receipt);
    assert_eq!(TerminalActions::capture(&state), first_actions);
}

#[test]
fn failed_rebind_preserves_a_concurrent_operation_failure() {
    let run = durable_run("terminal-recover-bind-and-operation-failure", Vec::new());
    run.rewind(CrashBoundary::IntentPrepared);
    let state = FakeHandle::new();
    {
        let mut state = state.0.borrow_mut();
        state.terminal.lose_bind_acknowledgement = true;
        state.terminal.failed_operations = vec![RunTerminalOperation::ControlStop];
    }

    let recovered = open_one(run.directory.path(), state.clone()).expect("recovery open");
    let receipt = latest_receipt(recovered.journal().entries());

    assert!(matches!(
        receipt.report().binding_status(),
        RunTerminalBindingStatus::Failed { .. }
    ));
    assert!(matches!(
        receipt.report().operations()[1].status(),
        RunTerminalOperationStatus::Failed { .. }
    ));
    assert_eq!(
        state.0.borrow().terminal.operation_order(),
        &ADAPTER_OPERATIONS
    );
}

#[test]
fn crash_after_terminal_report_rebinds_only_when_receipt_is_absent() {
    let run = durable_run("terminal-recover-report", Vec::new());
    assert_recovery(
        run,
        CrashBoundary::ReportRecorded,
        ReceiptSeed::Absent,
        ExpectedActions::report_seal(),
        true,
    );
}

#[test]
fn crash_after_receipt_publication_commits_without_rebind_or_reseal() {
    let run = durable_run("terminal-recover-receipt", Vec::new());
    assert_recovery(
        run,
        CrashBoundary::ReportRecorded,
        ReceiptSeed::Exact,
        ExpectedActions::exact_readback(),
        true,
    );
}

#[test]
fn crash_after_run_commit_closes_without_any_terminal_adapter_action() {
    let run = durable_run("terminal-recover-commit", Vec::new());
    assert_recovery(
        run,
        CrashBoundary::RunCommitted,
        ReceiptSeed::Absent,
        ExpectedActions::none(),
        true,
    );
}

#[test]
fn crash_after_quarantine_receipt_publication_keeps_the_exact_class() {
    let run = durable_run(
        "terminal-recover-quarantine-receipt",
        vec![RunTerminalOperation::ControlStop],
    );
    assert!(matches!(
        run.receipt.class().disposition(),
        flight_tune::RunTerminalDisposition::Quarantine {
            quarantine: RunTerminalQuarantine::TerminalFailure
        }
    ));
    assert_recovery(
        run,
        CrashBoundary::ReportRecorded,
        ReceiptSeed::Exact,
        ExpectedActions::exact_readback(),
        false,
    );
}

#[derive(Clone, Copy)]
enum ReceiptSeed {
    Absent,
    Exact,
}

struct ExpectedActions {
    simulator_stop: usize,
    bind: usize,
    operations: &'static [RunTerminalOperation],
    causal_read: usize,
    seal: usize,
    recover: usize,
}

impl ExpectedActions {
    const fn runtime_containment() -> Self {
        Self::new(0, 1, &ADAPTER_OPERATIONS, 1, 1, 2)
    }

    const fn active_containment() -> Self {
        Self::new(1, 1, &ADAPTER_OPERATIONS, 1, 1, 2)
    }

    const fn report_seal() -> Self {
        Self::new(0, 1, &[], 0, 1, 2)
    }

    const fn exact_readback() -> Self {
        Self::new(0, 0, &[], 0, 0, 1)
    }

    const fn none() -> Self {
        Self::new(0, 0, &[], 0, 0, 0)
    }

    const fn new(
        simulator_stop: usize,
        bind: usize,
        operations: &'static [RunTerminalOperation],
        causal_read: usize,
        seal: usize,
        recover: usize,
    ) -> Self {
        Self {
            simulator_stop,
            bind,
            operations,
            causal_read,
            seal,
            recover,
        }
    }

    fn assert_eq(&self, actual: &TerminalActions) {
        assert_eq!(actual.simulator_stop, self.simulator_stop);
        assert_eq!(actual.bind, self.bind);
        assert_eq!(actual.operations, self.operations);
        assert_eq!(actual.causal_read, self.causal_read);
        assert_eq!(actual.seal, self.seal);
        assert_eq!(actual.recover, self.recover);
    }
}

fn assert_recovery(
    run: DurableRun,
    boundary: CrashBoundary,
    receipt_seed: ReceiptSeed,
    expected: ExpectedActions,
    completed: bool,
) {
    run.rewind(boundary);
    let state = FakeHandle::new();
    if matches!(receipt_seed, ReceiptSeed::Exact) {
        state.0.borrow_mut().terminal.recovery_receipts = Some(vec![run.receipt.clone()]);
    }

    let first = open_one(run.directory.path(), state.clone()).expect("first recovery open");
    let recovered = latest_receipt(first.journal().entries()).clone();
    assert_eq!(recovered.is_completed(), completed);
    assert_recovery_semantic(&recovered, boundary);
    assert_one_attempt_closure(first.journal().entries());
    let first_actions = TerminalActions::capture(&state);
    expected.assert_eq(&first_actions);
    drop(first);

    let second = open_one(run.directory.path(), state.clone()).expect("second recovery open");
    assert_eq!(latest_receipt(second.journal().entries()), &recovered);
    assert_one_attempt_closure(second.journal().entries());
    assert_eq!(TerminalActions::capture(&state), first_actions);
}

fn assert_recovery_semantic(receipt: &flight_tune::RunTerminalReceipt, boundary: CrashBoundary) {
    if matches!(
        boundary,
        CrashBoundary::RunPrepared | CrashBoundary::RunBound
    ) {
        assert!(matches!(
            receipt.intent().outcome(),
            RunTerminalSemanticOutcome::Recovery
        ));
    }
}

fn assert_one_attempt_closure(entries: &[flight_tune::JournalEntry]) {
    let closures = entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.event,
                JournalEvent::AttemptCompleted { .. } | JournalEvent::AttemptQuarantined { .. }
            )
        })
        .count();
    assert_eq!(closures, 1);
}
