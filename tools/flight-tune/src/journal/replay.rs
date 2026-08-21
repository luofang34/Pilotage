use crate::journal::{
    AttemptRole, CampaignPhase, FinalQualificationOutcome, JOURNAL_SCHEMA_VERSION, JournalEntry,
    JournalEvent, OperationStatus, PromotionDecision,
};
use crate::{CandidateEvaluation, Digest, SearchStage, TrainingObservation, TuneError};

mod plan;

#[derive(Debug, Clone)]
pub(crate) struct PendingAttempt {
    pub(crate) trial_id: u64,
    pub(crate) role: AttemptRole,
    pub(crate) candidate: Digest,
    pub(crate) outcome: Option<PendingOutcome>,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingOutcome {
    pub(crate) evaluation: CandidateEvaluation,
    pub(crate) selected: Option<bool>,
}

#[derive(Debug, Clone)]
pub(crate) struct JournalState {
    pub(crate) phase: CampaignPhase,
    pub(crate) training_incumbent: Digest,
    pub(crate) training_incumbent_evaluation: Option<CandidateEvaluation>,
    pub(crate) training_baseline: Option<CandidateEvaluation>,
    pub(crate) training_attempt_count: u64,
    pub(crate) training_history: Vec<TrainingObservation>,
    pub(crate) next_trial_id: u64,
    pub(crate) pending: Option<PendingAttempt>,
    pub(crate) frozen_candidate: Option<Digest>,
    pub(crate) promotion_baseline: Option<CandidateEvaluation>,
    pub(crate) promotion_frozen: Option<CandidateEvaluation>,
    pub(crate) promotion_decision: Option<PromotionDecision>,
    pub(crate) final_evaluation: Option<CandidateEvaluation>,
    pub(crate) final_outcome: Option<FinalQualificationOutcome>,
}

impl JournalState {
    pub(crate) fn pending_role(&self, trial_id: u64) -> Result<AttemptRole, TuneError> {
        self.pending
            .as_ref()
            .filter(|pending| pending.trial_id == trial_id && pending.outcome.is_none())
            .map(|pending| pending.role)
            .ok_or_else(|| invalid("the trial is not pending or already has an outcome"))
    }

    pub(crate) fn selected_release_candidate(&self, initial: Digest) -> Digest {
        if self
            .promotion_decision
            .as_ref()
            .is_some_and(PromotionDecision::is_promoted)
        {
            self.frozen_candidate.unwrap_or(initial)
        } else {
            initial
        }
    }
}

pub(super) fn replay(
    entries: &[JournalEntry],
    entry_digests: &[Digest],
    stage: &SearchStage,
) -> Result<JournalState, TuneError> {
    let Some(first) = entries.first() else {
        return Err(invalid("the journal is empty"));
    };
    if entries.len() != entry_digests.len() {
        return Err(invalid("the journal digest count does not match"));
    }
    validate_header(first, 0, None, &first.session)?;
    let JournalEvent::Started { candidate } = first.event else {
        return Err(invalid("the first event is not a start event"));
    };
    if candidate != first.session.initial_candidate_digest {
        return Err(invalid("the initial candidate digest does not match"));
    }
    let mut state = initial_state(candidate);
    for (index, entry) in entries.iter().enumerate().skip(1) {
        let sequence = u64::try_from(index).map_err(|_| invalid("sequence overflow"))?;
        validate_header(
            entry,
            sequence,
            Some(entry_digests[index - 1]),
            &first.session,
        )?;
        apply_event(
            &mut state,
            &entry.event,
            stage,
            candidate,
            first.session.fixed_seed,
        )?;
    }
    Ok(state)
}

fn initial_state(candidate: Digest) -> JournalState {
    JournalState {
        phase: CampaignPhase::Searching,
        training_incumbent: candidate,
        training_incumbent_evaluation: None,
        training_baseline: None,
        training_attempt_count: 0,
        training_history: Vec::new(),
        next_trial_id: 0,
        pending: None,
        frozen_candidate: None,
        promotion_baseline: None,
        promotion_frozen: None,
        promotion_decision: None,
        final_evaluation: None,
        final_outcome: None,
    }
}

fn validate_header(
    entry: &JournalEntry,
    sequence: u64,
    previous: Option<Digest>,
    session: &crate::SessionIdentity,
) -> Result<(), TuneError> {
    if entry.schema_version != JOURNAL_SCHEMA_VERSION
        || entry.sequence != sequence
        || entry.previous != previous
        || &entry.session != session
    {
        return Err(invalid(format!(
            "journal entry {} has the wrong header",
            entry.sequence
        )));
    }
    Ok(())
}

fn apply_event(
    state: &mut JournalState,
    event: &JournalEvent,
    stage: &SearchStage,
    initial: Digest,
    fixed_seed: u64,
) -> Result<(), TuneError> {
    match event {
        JournalEvent::Started { .. } => Err(invalid("the journal has two start events")),
        JournalEvent::AttemptPrepared {
            trial_id,
            role,
            candidate,
            plan_digest,
        } => prepare(
            state,
            *trial_id,
            *role,
            *candidate,
            *plan_digest,
            stage,
            initial,
            fixed_seed,
        ),
        JournalEvent::AttemptCompleted {
            trial_id,
            evaluation,
            selected_as_training_incumbent,
        } => complete(
            state,
            *trial_id,
            evaluation,
            *selected_as_training_incumbent,
            stage,
            fixed_seed,
        ),
        JournalEvent::AttemptQuarantined { trial_id, reason } => {
            quarantine(state, *trial_id, reason)
        }
        JournalEvent::CleanupRecorded {
            trial_id,
            stop,
            cleanup: cleanup_status,
        } => cleanup(state, *trial_id, stop, cleanup_status),
        JournalEvent::Frozen {
            baseline,
            candidate,
        } => close::freeze(state, *baseline, *candidate, initial),
        JournalEvent::PromotionClosed { decision } => close::promotion(state, decision, stage),
        JournalEvent::Sealed { candidate, outcome } => {
            close::seal(state, *candidate, outcome, initial, stage)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn prepare(
    state: &mut JournalState,
    trial_id: u64,
    role: AttemptRole,
    candidate: Digest,
    plan_digest: Digest,
    stage: &SearchStage,
    initial: Digest,
    fixed_seed: u64,
) -> Result<(), TuneError> {
    let expected_plan = role.plan_digest(stage, candidate, fixed_seed)?;
    if state.pending.is_some()
        || trial_id != state.next_trial_id
        || candidate.is_zero()
        || plan_digest != expected_plan
        || !role_allowed(state, role, candidate, initial)
    {
        return Err(invalid(
            "an attempt preparation has invalid state or identity",
        ));
    }
    state.pending = Some(PendingAttempt {
        trial_id,
        role,
        candidate,
        outcome: None,
    });
    state.next_trial_id = state.next_trial_id.wrapping_add(1);
    Ok(())
}

fn role_allowed(
    state: &JournalState,
    role: AttemptRole,
    candidate: Digest,
    initial: Digest,
) -> bool {
    match role {
        AttemptRole::TrainingBaseline => {
            state.phase == CampaignPhase::Searching
                && state.training_baseline.is_none()
                && candidate == initial
        }
        AttemptRole::TrainingChallenger { attempt_index } => {
            state.phase == CampaignPhase::Searching
                && state.training_baseline.is_some()
                && attempt_index == state.training_attempt_count
        }
        AttemptRole::PromotionBaseline => {
            state.phase == CampaignPhase::Frozen
                && state.promotion_baseline.is_none()
                && candidate == initial
        }
        AttemptRole::PromotionFrozen => {
            state.phase == CampaignPhase::Frozen
                && state.promotion_baseline.is_some()
                && state.promotion_frozen.is_none()
                && state.frozen_candidate == Some(candidate)
        }
        AttemptRole::FinalQualification => {
            state.phase == CampaignPhase::PromotionClosed
                && state.final_evaluation.is_none()
                && candidate == state.selected_release_candidate(initial)
        }
    }
}

fn complete(
    state: &mut JournalState,
    trial_id: u64,
    evaluation: &CandidateEvaluation,
    selected: Option<bool>,
    stage: &SearchStage,
    fixed_seed: u64,
) -> Result<(), TuneError> {
    let role = state
        .pending
        .as_ref()
        .filter(|pending| pending.trial_id == trial_id && pending.outcome.is_none())
        .map(|pending| pending.role)
        .ok_or_else(|| invalid("the attempt is not pending or already has an outcome"))?;
    plan::validate_evaluation(evaluation, role, stage, fixed_seed)?;
    validate_training_selection(state, role, evaluation, selected)?;
    let pending = pending_without_outcome(state, trial_id)?;
    pending.outcome = Some(PendingOutcome {
        evaluation: evaluation.clone(),
        selected,
    });
    Ok(())
}

fn validate_training_selection(
    state: &JournalState,
    role: AttemptRole,
    evaluation: &CandidateEvaluation,
    selected: Option<bool>,
) -> Result<(), TuneError> {
    let expected = match role {
        AttemptRole::TrainingBaseline => Some(evaluation.aggregate().is_some()),
        AttemptRole::TrainingChallenger { .. } => Some(training_better(
            state.training_incumbent_evaluation.as_ref(),
            evaluation,
        )),
        AttemptRole::PromotionBaseline
        | AttemptRole::PromotionFrozen
        | AttemptRole::FinalQualification => None,
    };
    if selected != expected {
        return Err(invalid("a training incumbent decision is not reproducible"));
    }
    Ok(())
}

fn training_better(
    incumbent: Option<&CandidateEvaluation>,
    challenger: &CandidateEvaluation,
) -> bool {
    let Some(challenger_score) = challenger.aggregate() else {
        return false;
    };
    incumbent
        .and_then(CandidateEvaluation::aggregate)
        .is_none_or(|incumbent_score| challenger_score.mean_loss < incumbent_score.mean_loss)
}

fn quarantine(state: &mut JournalState, trial_id: u64, reason: &str) -> Result<(), TuneError> {
    if reason.trim().is_empty() || reason.len() > 4_096 {
        return Err(invalid("a quarantine reason is empty or too long"));
    }
    let pending = pending_without_outcome(state, trial_id)?;
    let selected = match pending.role {
        AttemptRole::TrainingBaseline | AttemptRole::TrainingChallenger { .. } => Some(false),
        _ => None,
    };
    pending.outcome = Some(PendingOutcome {
        evaluation: CandidateEvaluation::Quarantined {
            reason: reason.to_owned(),
        },
        selected,
    });
    Ok(())
}

fn cleanup(
    state: &mut JournalState,
    trial_id: u64,
    stop: &OperationStatus,
    cleanup: &OperationStatus,
) -> Result<(), TuneError> {
    validate_operation_status(stop)?;
    validate_operation_status(cleanup)?;
    let Some(pending) = state.pending.as_ref() else {
        return Err(invalid("cleanup has no pending attempt"));
    };
    if pending.trial_id != trial_id || pending.outcome.is_none() {
        return Err(invalid("cleanup has the wrong trial or no saved outcome"));
    }
    if cleanup.succeeded() {
        let completed = state
            .pending
            .take()
            .ok_or_else(|| invalid("cleanup lost its pending attempt"))?;
        finalize(state, completed)?;
    }
    Ok(())
}

fn finalize(state: &mut JournalState, pending: PendingAttempt) -> Result<(), TuneError> {
    let outcome = pending
        .outcome
        .ok_or_else(|| invalid("a clean attempt has no outcome"))?;
    match pending.role {
        AttemptRole::TrainingBaseline => finalize_baseline(state, outcome),
        AttemptRole::TrainingChallenger { attempt_index } => {
            finalize_training(state, pending.candidate, attempt_index, outcome)
        }
        AttemptRole::PromotionBaseline => {
            state.promotion_baseline = Some(outcome.evaluation);
            Ok(())
        }
        AttemptRole::PromotionFrozen => {
            state.promotion_frozen = Some(outcome.evaluation);
            Ok(())
        }
        AttemptRole::FinalQualification => {
            state.final_evaluation = Some(outcome.evaluation);
            Ok(())
        }
    }
}

fn finalize_baseline(state: &mut JournalState, outcome: PendingOutcome) -> Result<(), TuneError> {
    if outcome.selected == Some(true) {
        state.training_incumbent_evaluation = Some(outcome.evaluation.clone());
    }
    state.training_baseline = Some(outcome.evaluation);
    Ok(())
}

fn finalize_training(
    state: &mut JournalState,
    candidate: Digest,
    attempt_index: u64,
    outcome: PendingOutcome,
) -> Result<(), TuneError> {
    if attempt_index != state.training_attempt_count {
        return Err(invalid("a completed training attempt has the wrong index"));
    }
    let selected = outcome.selected == Some(true);
    let observation = TrainingObservation {
        attempt_index,
        candidate_digest: candidate,
        selected_as_incumbent: selected,
        hard_gate_failed: matches!(
            outcome.evaluation,
            CandidateEvaluation::HardGateFailed { .. }
        ),
        quarantined: matches!(outcome.evaluation, CandidateEvaluation::Quarantined { .. }),
        training_mean_loss: outcome
            .evaluation
            .aggregate()
            .map(|aggregate| aggregate.mean_loss),
    };
    if selected {
        state.training_incumbent = candidate;
        state.training_incumbent_evaluation = Some(outcome.evaluation.clone());
    }
    state.training_history.push(observation);
    state.training_attempt_count = state.training_attempt_count.wrapping_add(1);
    Ok(())
}

fn pending_without_outcome(
    state: &mut JournalState,
    trial_id: u64,
) -> Result<&mut PendingAttempt, TuneError> {
    state
        .pending
        .as_mut()
        .filter(|pending| pending.trial_id == trial_id && pending.outcome.is_none())
        .ok_or_else(|| invalid("the attempt is not pending or already has an outcome"))
}

fn validate_operation_status(status: &OperationStatus) -> Result<(), TuneError> {
    if let OperationStatus::Failed { detail } = status
        && detail.trim().is_empty()
    {
        return Err(invalid("a cleanup failure detail is empty"));
    }
    Ok(())
}

fn invalid(detail: impl Into<String>) -> TuneError {
    TuneError::InvalidJournal {
        detail: detail.into(),
    }
}
mod close;
