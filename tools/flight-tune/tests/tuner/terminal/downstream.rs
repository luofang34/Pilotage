use std::fs;

use flight_tune::{
    ArtifactIdentity, AttemptRole, CampaignPhase, EvaluatorError, FinalQualificationOutcome,
    GateEvaluator, GateOutcome, JournalEntry, JournalEvent, MissionReference, PromotionDecision,
    RunTerminalDisposition, RunTerminalOperation, RunTerminalQuarantine, RunTerminalReceipt,
    TelemetrySample, TuneError, Tuner,
};

use crate::open_stage;
use crate::test_rig::{
    FakeBackend, FakeFactory, FakeHandle, FakeVehicle, QuadraticMetric, SequenceStrategy,
    TestDirectory, candidate, stage,
};

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

    let expected = MutationSnapshot::capture(tuner.journal().entries(), directory.path(), &state);
    for _ in 0..2 {
        assert!(tuner.run_final_qualification_once_blocking().is_err());
        assert_eq!(
            MutationSnapshot::capture(tuner.journal().entries(), directory.path(), &state),
            expected
        );
        assert_eq!(tuner.journal().phase(), CampaignPhase::PromotionClosed);
    }
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
    assert!(tuner.journal().verified_evidence_snapshot().is_err());

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

#[test]
fn successful_promotion_hard_gate_rejects_final_without_any_mutation() {
    let directory = TestDirectory::new("promotion-hard-gate-final-authorization");
    let state = FakeHandle::new();
    let mut tuner = open_with_promotion_gate(directory.path(), state.clone())
        .expect("open hard-gate promotion campaign");
    tuner
        .run_training_attempts_blocking(0)
        .expect("complete training baseline");
    tuner.freeze_candidate().expect("freeze candidate");

    assert!(matches!(
        tuner.run_promotion_once_blocking().expect("close promotion"),
        PromotionDecision::RejectedHardGate { ref gate_id } if gate_id == "envelope"
    ));
    let expected = MutationSnapshot::capture(tuner.journal().entries(), directory.path(), &state);
    for _ in 0..2 {
        assert!(tuner.run_final_qualification_once_blocking().is_err());
        assert_eq!(
            MutationSnapshot::capture(tuner.journal().entries(), directory.path(), &state),
            expected
        );
    }
}

#[test]
fn unsuccessful_hard_gate_terminal_report_becomes_indeterminate() {
    let directory = TestDirectory::new("promotion-hard-gate-terminal-failure");
    let state = FakeHandle::new();
    let mut tuner = open_with_promotion_gate(directory.path(), state.clone())
        .expect("open hard-gate promotion campaign");
    tuner
        .run_training_attempts_blocking(0)
        .expect("complete training baseline");
    tuner.freeze_candidate().expect("freeze candidate");
    state.0.borrow_mut().terminal.failed_operations = vec![RunTerminalOperation::ControlStop];

    assert!(tuner.run_promotion_once_blocking().is_err());
    assert!(matches!(
        tuner
            .run_promotion_once_blocking()
            .expect("close quarantined promotion"),
        PromotionDecision::Indeterminate { .. }
    ));
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

type PromotionGateTuner =
    Tuner<FakeBackend, FakeVehicle, PromotionGate, QuadraticMetric, SequenceStrategy>;

fn open_with_promotion_gate(
    path: &std::path::Path,
    state: FakeHandle,
) -> Result<PromotionGateTuner, TuneError> {
    Tuner::open_or_resume(
        path,
        stage(),
        91,
        candidate(0.0),
        FakeBackend::new(state.clone()),
        FakeFactory::new(state.clone()),
        PromotionGate::new(state.clone()),
        QuadraticMetric::new(state),
        SequenceStrategy::new(Vec::new()),
    )
}

struct PromotionGate {
    identity: ArtifactIdentity,
    state: FakeHandle,
    fail_current: bool,
}

impl PromotionGate {
    fn new(state: FakeHandle) -> Self {
        Self {
            identity: ArtifactIdentity::from_text("gates", "fail-promotion-envelope")
                .expect("gate identity"),
            state,
            fail_current: false,
        }
    }
}

impl GateEvaluator for PromotionGate {
    fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    fn begin(&mut self, scenario: &MissionReference) -> Result<(), EvaluatorError> {
        self.fail_current = scenario.revision_id.starts_with("promotion-");
        let mut state = self.state.0.borrow_mut();
        state.gate_begin_count = state.gate_begin_count.wrapping_add(1);
        Ok(())
    }

    fn evaluate(&mut self, _sample: &TelemetrySample) -> Result<Vec<GateOutcome>, EvaluatorError> {
        let mut state = self.state.0.borrow_mut();
        state.gate_evaluate_count = state.gate_evaluate_count.wrapping_add(1);
        drop(state);
        if self.fail_current {
            Ok(vec![GateOutcome::fail(
                "envelope",
                "promotion envelope failed",
            )])
        } else {
            Ok(vec![GateOutcome::pass("envelope")])
        }
    }

    fn finish(&mut self) -> Result<(), EvaluatorError> {
        let mut state = self.state.0.borrow_mut();
        state.gate_finish_count = state.gate_finish_count.wrapping_add(1);
        Ok(())
    }

    fn cancel(&mut self) -> Result<(), EvaluatorError> {
        let mut state = self.state.0.borrow_mut();
        state.gate_cancel_count = state.gate_cancel_count.wrapping_add(1);
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
struct MutationSnapshot {
    entries: Vec<JournalEntry>,
    head: Vec<u8>,
    external_state: String,
}

impl MutationSnapshot {
    fn capture(entries: &[JournalEntry], root: &std::path::Path, state: &FakeHandle) -> Self {
        Self {
            entries: entries.to_vec(),
            head: fs::read(root.join("HEAD.json")).expect("read journal head"),
            external_state: format!("{:?}", *state.0.borrow()),
        }
    }
}
