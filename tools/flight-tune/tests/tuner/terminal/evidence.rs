use flight_tune::{
    ArtifactIdentity, Digest, JournalEvent, RunBindingReceipt, RunTerminalClass,
    RunTerminalDisposition, RunTerminalOperation, RunTerminalQuarantine, RunTerminalReceipt,
    TuneError,
};

use crate::test_rig::{
    FakeHandle, FakeTerminalReadbackFault, FakeTerminalSealFault, TestDirectory,
};

use super::{CrashBoundary, DurableRun, TerminalActions, durable_run, latest_receipt, open_one};

#[test]
fn completed_seal_failure_before_publication_creates_one_quarantine_receipt() {
    let directory = TestDirectory::new("terminal-evidence-absent");
    let state = FakeHandle::new();
    state.0.borrow_mut().terminal.seal_fault = FakeTerminalSealFault::FailBeforePublication;
    let mut tuner = open_one(directory.path(), state.clone()).expect("open evidence run");

    assert!(tuner.run_training_attempts_blocking(0).is_err());

    let receipt = latest_receipt(tuner.journal().entries());
    assert_evidence_failure(receipt);
    let terminal = &state.0.borrow().terminal;
    assert_eq!(terminal.seal_count(), 2);
    assert_eq!(terminal.recover_count(), 3);
    assert_eq!(
        terminal.receipts(receipt.binding().receipt_digest()),
        std::slice::from_ref(receipt)
    );
    assert_evidence_event_precedes_commit(tuner.journal().entries());
}

#[test]
fn lost_completed_seal_acknowledgement_accepts_exact_readback() {
    let directory = TestDirectory::new("terminal-evidence-lost-ack");
    let state = FakeHandle::new();
    state.0.borrow_mut().terminal.seal_fault = FakeTerminalSealFault::LoseAcknowledgement;
    let mut tuner = open_one(directory.path(), state.clone()).expect("open evidence run");

    assert!(tuner.run_training_attempts_blocking(0).is_err());

    let receipt = latest_receipt(tuner.journal().entries());
    assert!(receipt.is_completed());
    let terminal = &state.0.borrow().terminal;
    assert_eq!(terminal.seal_count(), 1);
    assert_eq!(terminal.recover_count(), 2);
    assert_eq!(
        terminal.receipts(receipt.binding().receipt_digest()),
        std::slice::from_ref(receipt)
    );
}

#[test]
fn lost_acknowledgement_with_changed_readback_poisons_the_session() {
    let directory = TestDirectory::new("terminal-evidence-changed-ack");
    let state = FakeHandle::new();
    {
        let mut state = state.0.borrow_mut();
        state.terminal.seal_fault = FakeTerminalSealFault::LoseAcknowledgement;
        state.terminal.readback_fault = FakeTerminalReadbackFault::ChangeReceipt;
    }
    let mut tuner = open_one(directory.path(), state).expect("open evidence run");

    let error = tuner
        .run_training_attempts_blocking(0)
        .expect_err("reject changed receipt readback");

    assert_poisoned_without_commit(&mut tuner, error);
}

#[test]
fn completed_intent_rejects_one_quarantine_receipt() {
    let completed = durable_run("terminal-source-completed", Vec::new()).receipt;
    let quarantine = evidence_failure_receipt(&completed);
    let directory = TestDirectory::new("terminal-completed-intent-quarantine-receipt");
    let state = FakeHandle::new();
    state.0.borrow_mut().terminal.recovery_receipts = Some(vec![quarantine]);
    let mut tuner = open_one(directory.path(), state).expect("open evidence run");

    let error = tuner
        .run_training_attempts_blocking(0)
        .expect_err("reject quarantine receipt for completed intent");

    assert_poisoned_without_commit(&mut tuner, error);
}

#[test]
fn quarantine_intent_rejects_one_completed_receipt() {
    let completed = durable_run("terminal-source-for-quarantine", Vec::new()).receipt;
    let directory = TestDirectory::new("terminal-quarantine-intent-completed-receipt");
    let state = FakeHandle::new();
    {
        let mut state = state.0.borrow_mut();
        state.terminal.failed_operations = vec![RunTerminalOperation::ControlStop];
        state.terminal.recovery_receipts = Some(vec![completed]);
    }
    let mut tuner = open_one(directory.path(), state).expect("open evidence run");

    let error = tuner
        .run_training_attempts_blocking(0)
        .expect_err("reject completed receipt for quarantine intent");

    assert_poisoned_without_commit(&mut tuner, error);
}

#[test]
fn two_terminal_receipt_classes_poison_the_session() {
    let completed = durable_run("terminal-source-two-classes", Vec::new()).receipt;
    let quarantine = evidence_failure_receipt(&completed);
    let directory = TestDirectory::new("terminal-two-receipt-classes");
    let state = FakeHandle::new();
    state.0.borrow_mut().terminal.recovery_receipts = Some(vec![completed.clone(), quarantine]);
    let mut tuner = open_one(directory.path(), state).expect("open evidence run");

    let error = tuner
        .run_training_attempts_blocking(0)
        .expect_err("reject two receipt classes");

    assert_poisoned_without_commit(&mut tuner, error);
}

#[test]
fn absent_receipt_for_quarantine_report_fails_closed() {
    let directory = TestDirectory::new("terminal-quarantine-receipt-absent");
    let state = FakeHandle::new();
    {
        let mut state = state.0.borrow_mut();
        state.terminal.failed_operations = vec![RunTerminalOperation::ControlStop];
        state.terminal.seal_fault = FakeTerminalSealFault::FailBeforePublication;
    }
    let mut tuner = open_one(directory.path(), state.clone()).expect("open evidence run");

    assert!(tuner.run_training_attempts_blocking(0).is_err());

    assert!(
        tuner
            .journal()
            .entries()
            .iter()
            .any(|entry| matches!(entry.event, JournalEvent::RunTerminalReportRecorded { .. }))
    );
    assert!(tuner.journal().entries().iter().all(|entry| !matches!(
        entry.event,
        JournalEvent::RunCommitted { .. }
            | JournalEvent::AttemptCompleted { .. }
            | JournalEvent::AttemptQuarantined { .. }
    )));
    let terminal = &state.0.borrow().terminal;
    assert_eq!(terminal.seal_count(), 1);
    assert_eq!(terminal.recover_count(), 2);
}

#[test]
fn successful_seal_with_empty_readback_does_not_create_evidence_failure() {
    let directory = TestDirectory::new("terminal-successful-seal-empty");
    let state = FakeHandle::new();
    state.0.borrow_mut().terminal.seal_fault = FakeTerminalSealFault::SucceedWithoutPublication;
    let mut tuner = open_one(directory.path(), state.clone()).expect("open evidence run");

    let error = tuner
        .run_training_attempts_blocking(0)
        .expect_err("reject an acknowledged seal with empty readback");

    assert!(matches!(
        error,
        TuneError::InvalidState {
            operation: "seal terminal receipt",
            ..
        }
    ));
    let expected = report_authority(tuner.journal().entries()).clone();
    assert_no_evidence_failure(tuner.journal().entries());
    assert!(
        tuner
            .journal()
            .entries()
            .iter()
            .all(|entry| !matches!(entry.event, JournalEvent::RunCommitted { .. }))
    );
    drop(tuner);

    let recovered = open_one(directory.path(), state.clone()).expect("recover exact authority");
    assert_eq!(latest_receipt(recovered.journal().entries()), &expected);
    assert!(latest_receipt(recovered.journal().entries()).is_completed());
    assert_no_evidence_failure(recovered.journal().entries());
    let actions = TerminalActions::capture(&state);
    drop(recovered);

    let reopened = open_one(directory.path(), state.clone()).expect("reopen committed run");
    assert_eq!(latest_receipt(reopened.journal().entries()), &expected);
    assert_eq!(TerminalActions::capture(&state), actions);
}

#[test]
fn transient_receipt_read_error_does_not_poison_or_repeat_terminal_actions() {
    let directory = TestDirectory::new("terminal-transient-read-error");
    let state = FakeHandle::new();
    state.0.borrow_mut().terminal.readback_fault = FakeTerminalReadbackFault::ReturnError;
    let mut tuner = open_one(directory.path(), state.clone()).expect("open evidence run");

    let error = tuner
        .run_training_attempts_blocking(0)
        .expect_err("return transient receipt read error");

    assert!(matches!(
        error,
        TuneError::Adapter {
            operation: "recover terminal receipts",
            ..
        }
    ));
    let before_retry = TerminalActions::capture(&state);
    assert_eq!(before_retry.seal, 0);
    assert!(tuner.run_training_attempts_blocking(0).is_err());
    let receipt = latest_receipt(tuner.journal().entries()).clone();
    assert!(receipt.is_completed());
    let after_retry = TerminalActions::capture(&state);
    assert_eq!(after_retry.operations, before_retry.operations);
    assert_eq!(after_retry.causal_read, before_retry.causal_read);
    drop(tuner);

    let reopened = open_one(directory.path(), state.clone()).expect("reopen recovered run");
    assert_eq!(latest_receipt(reopened.journal().entries()), &receipt);
    assert_eq!(TerminalActions::capture(&state), after_retry);
}

#[test]
fn execution_and_terminal_failures_are_both_returned() {
    let directory = TestDirectory::new("terminal-primary-and-terminal-errors");
    let state = FakeHandle::new();
    {
        let mut state = state.0.borrow_mut();
        state.vehicle.bad_candidate_readback_on_ensure = Some(2);
        state.terminal.readback_fault = FakeTerminalReadbackFault::ReturnError;
    }
    let mut tuner = open_one(directory.path(), state).expect("open evidence run");

    let error = tuner
        .run_training_attempts_blocking(0)
        .expect_err("return both failures");

    let TuneError::OperationAndTerminalFailed {
        operation,
        primary,
        terminal,
    } = error
    else {
        panic!("combined failure was not returned");
    };
    assert_eq!(operation, "evaluate candidate");
    assert!(matches!(
        primary.as_ref(),
        TuneError::ReceiptMismatch {
            operation: "ensure candidate",
            ..
        }
    ));
    assert!(matches!(
        terminal.as_ref(),
        TuneError::Adapter {
            operation: "recover terminal receipts",
            ..
        }
    ));
    assert!(!matches!(
        tuner.run_training_attempts_blocking(0),
        Err(TuneError::JournalPoisoned)
    ));
}

#[test]
fn changed_causal_receipt_poisons_each_reopen_without_external_action() {
    let run = durable_run("terminal-changed-causal-reopen", Vec::new());
    let changed = changed_causal_receipt(&run.receipt);
    assert_corrupt_receipts_on_two_opens(run, vec![changed]);
}

#[test]
fn malformed_receipt_poisons_each_reopen_without_external_action() {
    let run = durable_run("terminal-malformed-reopen", Vec::new());
    let malformed = malformed_receipt(run.receipt.clone());
    assert_corrupt_receipts_on_two_opens(run, vec![malformed]);
}

#[test]
fn foreign_receipt_poisons_each_reopen_without_external_action() {
    let run = durable_run("terminal-foreign-reopen", Vec::new());
    let foreign = foreign_receipt(&run.receipt);
    assert_corrupt_receipts_on_two_opens(run, vec![foreign]);
}

fn evidence_failure_receipt(completed: &RunTerminalReceipt) -> RunTerminalReceipt {
    let class = RunTerminalClass::evidence_failure(completed.intent(), completed.report())
        .expect("classify evidence failure");
    RunTerminalReceipt::new(
        completed.binding(),
        completed.intent(),
        completed.report(),
        class,
        completed.causal_evidence_digest(),
    )
    .expect("make evidence failure receipt")
}

fn changed_causal_receipt(receipt: &RunTerminalReceipt) -> RunTerminalReceipt {
    RunTerminalReceipt::new(
        receipt.binding(),
        receipt.intent(),
        receipt.report(),
        receipt.class(),
        Digest::from_bytes([97; 32]),
    )
    .expect("make changed causal receipt")
}

fn malformed_receipt(receipt: RunTerminalReceipt) -> RunTerminalReceipt {
    let mut document = serde_json::to_value(receipt).expect("encode terminal receipt");
    document["receipt_digest"] =
        serde_json::to_value(Digest::from_bytes([98; 32])).expect("encode changed digest");
    serde_json::from_value(document).expect("decode malformed receipt")
}

fn foreign_receipt(receipt: &RunTerminalReceipt) -> RunTerminalReceipt {
    let adapter = ArtifactIdentity::new("foreign-terminal-adapter", Digest::from_bytes([99; 32]))
        .expect("make foreign adapter");
    let binding = RunBindingReceipt::new(receipt.context(), receipt.report().plan(), adapter)
        .expect("make foreign binding");
    RunTerminalReceipt::new(
        &binding,
        receipt.intent(),
        receipt.report(),
        receipt.class(),
        receipt.causal_evidence_digest(),
    )
    .expect("make foreign receipt")
}

fn assert_corrupt_receipts_on_two_opens(run: DurableRun, receipts: Vec<RunTerminalReceipt>) {
    run.rewind(CrashBoundary::ReportRecorded);
    let state = FakeHandle::new();
    state.0.borrow_mut().terminal.recovery_receipts = Some(receipts);
    for recover_count in 1..=2 {
        let result = open_one(run.directory.path(), state.clone());
        let Err(error) = result else {
            panic!("corrupt receipt resumed the campaign");
        };
        assert!(matches!(error, TuneError::ReceiptMismatch { .. }));
        let actions = TerminalActions::capture(&state);
        assert_eq!(actions.recover, recover_count);
        assert_eq!(actions.simulator_stop, 0);
        assert_eq!(actions.bind, 0);
        assert!(actions.operations.is_empty());
        assert_eq!(actions.causal_read, 0);
        assert_eq!(actions.seal, 0);
    }
}

fn report_authority(entries: &[flight_tune::JournalEntry]) -> &RunTerminalReceipt {
    entries
        .iter()
        .find_map(|entry| match &entry.event {
            JournalEvent::RunTerminalReportRecorded {
                expected_receipt, ..
            } => Some(expected_receipt.as_ref()),
            _ => None,
        })
        .expect("terminal report authority")
}

fn assert_no_evidence_failure(entries: &[flight_tune::JournalEntry]) {
    assert!(entries.iter().all(|entry| !matches!(
        entry.event,
        JournalEvent::RunTerminalEvidenceFailureRecorded { .. }
    )));
}

fn assert_poisoned_without_commit(tuner: &mut super::super::TestTuner, error: TuneError) {
    assert!(matches!(error, TuneError::ReceiptMismatch { .. }));
    assert!(
        tuner
            .journal()
            .entries()
            .iter()
            .all(|entry| !matches!(entry.event, JournalEvent::RunCommitted { .. }))
    );
    assert!(matches!(
        tuner.run_training_attempts_blocking(0),
        Err(TuneError::JournalPoisoned)
    ));
}

fn assert_evidence_failure(receipt: &RunTerminalReceipt) {
    assert_eq!(
        receipt.class().disposition(),
        RunTerminalDisposition::Quarantine {
            quarantine: RunTerminalQuarantine::EvidenceFailure,
        }
    );
}

fn assert_evidence_event_precedes_commit(entries: &[flight_tune::JournalEntry]) {
    let evidence = entries
        .iter()
        .position(|entry| {
            matches!(
                entry.event,
                JournalEvent::RunTerminalEvidenceFailureRecorded { .. }
            )
        })
        .expect("evidence failure event");
    let commit = entries
        .iter()
        .position(|entry| matches!(entry.event, JournalEvent::RunCommitted { .. }))
        .expect("terminal commit");
    assert!(evidence < commit);
}
