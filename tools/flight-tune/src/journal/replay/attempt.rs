use crate::journal::{AttemptRole, AuthenticatedEvaluationProof, CampaignPhase, OperationStatus};
use crate::{
    CandidateEvaluation, CandidateTransitionReference, Digest, SearchStage, TrainingObservation,
    TuneError,
};

use super::{JournalState, PendingAttempt, PendingOutcome, invalid, plan, terminal, transition};

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare(
    state: &mut JournalState,
    trial_id: u64,
    role: AttemptRole,
    candidate: Digest,
    plan_digest: Digest,
    transition_reference: Option<&CandidateTransitionReference>,
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
    transition::validate_attempt(state, role, candidate, transition_reference)?;
    state.pending = Some(PendingAttempt {
        trial_id,
        role,
        candidate,
        plan_digest,
        transition: transition_reference.cloned(),
        prepared_runs: Vec::new(),
        outcome: None,
    });
    state.authorized_transition = None;
    state.next_trial_id = state.next_trial_id.wrapping_add(1);
    Ok(())
}

pub(super) fn role_allowed(
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
                && has_passed_training_baseline_and_incumbent(state)
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
                && state
                    .authorized_final_candidate(initial)
                    .is_ok_and(|authorized| authorized == candidate)
        }
    }
}

pub(super) fn has_passed_training_baseline_and_incumbent(state: &JournalState) -> bool {
    matches!(
        &state.training_baseline,
        Some(CandidateEvaluation::Passed { .. })
    ) && matches!(
        &state.training_incumbent_evaluation,
        Some(CandidateEvaluation::Passed { .. })
    )
}

pub(super) fn complete(
    state: &mut JournalState,
    trial_id: u64,
    evaluation: &CandidateEvaluation,
    proof: Option<&AuthenticatedEvaluationProof>,
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
    let pending = state
        .pending
        .as_ref()
        .ok_or_else(|| invalid("the attempt lost its run preparation"))?;
    terminal::validate_completed_attempt(pending, evaluation)?;
    validate_proof(pending, evaluation, proof)?;
    validate_training_selection(state, role, evaluation, selected)?;
    let pending = pending_without_outcome(state, trial_id)?;
    pending.outcome = Some(PendingOutcome {
        evaluation: evaluation.clone(),
        selected,
        proof: proof.cloned(),
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

pub(super) fn quarantine(
    state: &mut JournalState,
    trial_id: u64,
    reason: &str,
    proof: Option<&AuthenticatedEvaluationProof>,
) -> Result<(), TuneError> {
    let pending = state
        .pending
        .as_ref()
        .filter(|pending| pending.trial_id == trial_id && pending.outcome.is_none())
        .ok_or_else(|| invalid("the attempt is not pending or already has an outcome"))?;
    terminal::validate_quarantined_attempt(pending, reason)?;
    let evaluation = CandidateEvaluation::Quarantined {
        reason: reason.to_owned(),
    };
    validate_proof(pending, &evaluation, proof)?;
    let pending = pending_without_outcome(state, trial_id)?;
    let selected = match pending.role {
        AttemptRole::TrainingBaseline | AttemptRole::TrainingChallenger { .. } => Some(false),
        _ => None,
    };
    pending.outcome = Some(PendingOutcome {
        evaluation,
        selected,
        proof: proof.cloned(),
    });
    Ok(())
}

pub(super) fn cleanup(
    state: &mut JournalState,
    trial_id: u64,
    cleanup: &OperationStatus,
) -> Result<(), TuneError> {
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
            state.promotion_baseline_proof = outcome.proof;
            Ok(())
        }
        AttemptRole::PromotionFrozen => {
            state.promotion_frozen = Some(outcome.evaluation);
            state.promotion_frozen_proof = outcome.proof;
            Ok(())
        }
        AttemptRole::FinalQualification => {
            state.final_evaluation = Some(outcome.evaluation);
            state.final_proof = outcome.proof;
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
    match status {
        OperationStatus::Succeeded => Ok(()),
        OperationStatus::Failed { detail } if !detail.trim().is_empty() => Ok(()),
        OperationStatus::Failed { .. } => Err(invalid("a cleanup failure detail is empty")),
        OperationStatus::NotRequired => Err(invalid("attempt cleanup is always required")),
    }
}

fn validate_proof(
    pending: &PendingAttempt,
    evaluation: &CandidateEvaluation,
    proof: Option<&AuthenticatedEvaluationProof>,
) -> Result<(), TuneError> {
    let required = matches!(
        pending.role,
        AttemptRole::PromotionBaseline
            | AttemptRole::PromotionFrozen
            | AttemptRole::FinalQualification
    );
    if !required {
        return if proof.is_none() {
            Ok(())
        } else {
            Err(invalid("a training attempt cannot contain a hidden proof"))
        };
    }
    let proof = proof.ok_or_else(|| invalid("a hidden attempt has no authenticated proof"))?;
    proof.validate()?;
    let receipts = terminal::owned_committed_receipts(pending)?;
    if proof.trial_id != pending.trial_id
        || proof.role != pending.role
        || proof.candidate_digest != pending.candidate
        || proof.plan_digest != pending.plan_digest
        || &proof.evaluation != evaluation
        || proof.terminal_receipts != receipts
    {
        return Err(invalid(
            "an authenticated proof changed its pending attempt",
        ));
    }
    Ok(())
}
