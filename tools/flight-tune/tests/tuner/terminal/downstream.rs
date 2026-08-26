use flight_tune::{
    AttemptRole, CampaignPhase, FinalQualificationOutcome, JournalEvent, PromotionDecision,
    RunTerminalDisposition, RunTerminalOperation, RunTerminalQuarantine, RunTerminalReceipt,
};

use crate::open_stage;
use crate::test_rig::{FakeHandle, SequenceStrategy, TestDirectory, stage};

use super::{TerminalActions, latest_receipt, open_with_gate};

#[test]
fn quarantined_promotion_receipt_can_only_close_as_indeterminate() {
    let directory = TestDirectory::new("terminal-quarantine-promotion");
    let state = FakeHandle::new();
    let mut tuner =
        open_with_gate(directory.path(), state.clone(), 2.0).expect("open promotion campaign");
    tuner
        .run_training_attempts_blocking(0)
        .expect("complete training baseline");
    tuner.freeze_candidate().expect("freeze candidate");
    state.0.borrow_mut().terminal.failed_operations = vec![RunTerminalOperation::ControlStop];

    assert!(tuner.run_promotion_once_blocking().is_err());

    let receipt = latest_receipt(tuner.journal().entries());
    assert_quarantine(receipt, AttemptRole::PromotionBaseline);
    assert_quarantined_without_completion(tuner.journal().entries());
    let actions = TerminalActions::capture(&state);
    let decision = tuner
        .run_promotion_once_blocking()
        .expect("close indeterminate promotion");
    assert!(matches!(decision, PromotionDecision::Indeterminate { .. }));
    assert!(!decision.is_promoted());
    assert_eq!(TerminalActions::capture(&state), actions);
    assert_eq!(tuner.journal().phase(), CampaignPhase::PromotionClosed);
}

#[test]
fn quarantined_final_receipt_cannot_create_a_qualified_result() {
    let directory = TestDirectory::new("terminal-quarantine-qualification");
    let state = FakeHandle::new();
    let mut tuner = open_stage(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(vec![0.5]),
        2.0,
        stage(),
    )
    .expect("open qualification campaign");
    tuner
        .run_training_attempts_blocking(1)
        .expect("select qualifying candidate");
    tuner.freeze_candidate().expect("freeze candidate");
    assert!(matches!(
        tuner.run_promotion_once_blocking().expect("run promotion"),
        PromotionDecision::Promoted { .. }
    ));
    state.0.borrow_mut().terminal.failed_operations = vec![RunTerminalOperation::ControlStop];

    assert!(tuner.run_final_qualification_once_blocking().is_err());

    let receipt = latest_receipt(tuner.journal().entries());
    assert_quarantine(receipt, AttemptRole::FinalQualification);
    assert_quarantined_without_completion(tuner.journal().entries());
    let actions = TerminalActions::capture(&state);
    let outcome = tuner
        .run_final_qualification_once_blocking()
        .expect("seal indeterminate qualification");
    assert!(matches!(
        outcome,
        FinalQualificationOutcome::Indeterminate { .. }
    ));
    assert_eq!(TerminalActions::capture(&state), actions);
    assert_eq!(tuner.journal().phase(), CampaignPhase::Sealed);
    assert!(tuner.qualified_candidate().is_err());
}

fn assert_quarantine(receipt: &RunTerminalReceipt, expected_role: AttemptRole) {
    assert_eq!(receipt.binding().context().role(), expected_role);
    assert_eq!(
        receipt.class().disposition(),
        RunTerminalDisposition::Quarantine {
            quarantine: RunTerminalQuarantine::TerminalFailure,
        }
    );
}

fn assert_quarantined_without_completion(entries: &[flight_tune::JournalEntry]) {
    assert!(
        entries
            .iter()
            .rev()
            .any(|entry| matches!(entry.event, JournalEvent::AttemptQuarantined { .. }))
    );
    let last_closure = entries.iter().rev().find(|entry| {
        matches!(
            entry.event,
            JournalEvent::AttemptCompleted { .. } | JournalEvent::AttemptQuarantined { .. }
        )
    });
    assert!(matches!(
        last_closure.map(|entry| &entry.event),
        Some(JournalEvent::AttemptQuarantined { .. })
    ));
}
