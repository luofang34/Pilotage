use crate::journal::replay::JournalState;
use crate::{
    CampaignPhase, CandidateEvaluation, Digest, FinalQualificationOutcome, PromotionDecision,
    SearchStage, TuneError,
};

pub(super) fn freeze(
    state: &mut JournalState,
    baseline: Digest,
    candidate: Digest,
    initial: Digest,
) -> Result<(), TuneError> {
    if state.phase != CampaignPhase::Searching
        || state.pending.is_some()
        || state
            .training_baseline
            .as_ref()
            .and_then(CandidateEvaluation::aggregate)
            .is_none()
        || baseline != initial
        || candidate != state.training_incumbent
    {
        return Err(invalid(
            "the frozen candidate does not match completed training",
        ));
    }
    state.phase = CampaignPhase::Frozen;
    state.frozen_candidate = Some(candidate);
    Ok(())
}

pub(super) fn promotion(
    state: &mut JournalState,
    decision: &PromotionDecision,
    _stage: &SearchStage,
) -> Result<(), TuneError> {
    if state.phase != CampaignPhase::Frozen
        || state.pending.is_some()
        || !promotion_shape_matches(state, decision)
    {
        return Err(invalid(
            "the promotion decision does not match hidden results",
        ));
    }
    state.phase = CampaignPhase::PromotionClosed;
    state.promotion_decision = Some(decision.clone());
    Ok(())
}

pub(super) fn seal(
    state: &mut JournalState,
    candidate: Digest,
    outcome: &FinalQualificationOutcome,
    initial: Digest,
) -> Result<(), TuneError> {
    if state.phase != CampaignPhase::PromotionClosed
        || state.pending.is_some()
        || candidate != state.selected_release_candidate(initial)
        || !final_shape_matches(state.final_evaluation.as_ref(), outcome)
    {
        return Err(invalid("the final seal does not match qualification"));
    }
    state.phase = CampaignPhase::Sealed;
    state.final_outcome = Some(outcome.clone());
    Ok(())
}

fn promotion_shape_matches(state: &JournalState, decision: &PromotionDecision) -> bool {
    let baseline = state.promotion_baseline.as_ref();
    let frozen = state.promotion_frozen.as_ref();
    match decision {
        PromotionDecision::Promoted { .. } | PromotionDecision::RejectedNoImprovement { .. } => {
            baseline.and_then(CandidateEvaluation::aggregate).is_some()
                && frozen.and_then(CandidateEvaluation::aggregate).is_some()
                && promotion_numbers_are_finite(decision)
        }
        PromotionDecision::RejectedHardGate { gate_id } => {
            !gate_id.trim().is_empty()
                && [baseline, frozen]
                    .into_iter()
                    .flatten()
                    .any(|evaluation| {
                        matches!(evaluation, CandidateEvaluation::HardGateFailed { failure, .. } if failure.gate.id == *gate_id)
                    })
        }
        PromotionDecision::Indeterminate { reason } => {
            !reason.trim().is_empty()
                && [baseline, frozen].into_iter().any(|evaluation| {
                    evaluation.is_none()
                        || matches!(evaluation, Some(CandidateEvaluation::Quarantined { .. }))
                })
        }
    }
}

fn promotion_numbers_are_finite(decision: &PromotionDecision) -> bool {
    match decision {
        PromotionDecision::Promoted {
            mean_loss_delta,
            loss_delta_upper_95,
            mean_effort_delta,
        } => {
            mean_loss_delta.is_finite()
                && loss_delta_upper_95.is_finite()
                && mean_effort_delta.is_finite()
        }
        PromotionDecision::RejectedNoImprovement {
            loss_delta_upper_95,
            mean_effort_delta,
        } => loss_delta_upper_95.is_finite() && mean_effort_delta.is_finite(),
        PromotionDecision::RejectedHardGate { .. } | PromotionDecision::Indeterminate { .. } => {
            true
        }
    }
}

fn final_shape_matches(
    evaluation: Option<&CandidateEvaluation>,
    outcome: &FinalQualificationOutcome,
) -> bool {
    match (evaluation, outcome) {
        (Some(CandidateEvaluation::Passed { .. }), FinalQualificationOutcome::Qualified) => true,
        (
            Some(CandidateEvaluation::HardGateFailed { failure, .. }),
            FinalQualificationOutcome::FailedHardGate { gate_id },
        ) => failure.gate.id == *gate_id,
        (
            Some(CandidateEvaluation::Quarantined { .. }) | None,
            FinalQualificationOutcome::Indeterminate { reason },
        ) => !reason.trim().is_empty(),
        _ => false,
    }
}

fn invalid(detail: impl Into<String>) -> TuneError {
    TuneError::InvalidJournal {
        detail: detail.into(),
    }
}
