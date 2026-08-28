use crate::journal::replay::JournalState;
use crate::journal::{AuthenticatedEvaluationProof, PromotionClosure, SessionIdentity};
use crate::{
    CampaignPhase, CandidateEvaluation, Digest, FinalQualificationOutcome, PromotionDecision,
    PromotionRunPlan, PromotionSelection, SearchStage, TuneError,
};

#[cfg(test)]
#[path = "close/tests.rs"]
mod tests;

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

pub(crate) fn expected_promotion_closure(
    state: &JournalState,
    stage: &SearchStage,
    session: &SessionIdentity,
) -> Result<PromotionClosure, TuneError> {
    if state.phase != CampaignPhase::Frozen || state.pending.is_some() {
        return Err(invalid("promotion cannot close in the current state"));
    }
    let baseline = state
        .promotion_baseline_proof
        .as_ref()
        .ok_or_else(|| invalid("promotion cannot close without an initial proof"))?;
    let frozen = state.promotion_frozen_proof.as_ref();
    validate_saved_proof(
        baseline,
        state.promotion_baseline.as_ref(),
        crate::AttemptRole::PromotionBaseline,
        session.initial_candidate_digest,
        stage,
        session,
    )?;
    validate_optional_frozen_proof(state, frozen, stage, session)?;
    build_promotion_closure(stage, session, baseline, frozen)
}

pub(crate) fn expected_promotion_closure_from_proofs(
    stage: &SearchStage,
    session: &SessionIdentity,
    baseline: &AuthenticatedEvaluationProof,
    frozen: Option<&AuthenticatedEvaluationProof>,
) -> Result<PromotionClosure, TuneError> {
    validate_saved_proof(
        baseline,
        Some(&baseline.evaluation),
        crate::AttemptRole::PromotionBaseline,
        session.initial_candidate_digest,
        stage,
        session,
    )?;
    match (&baseline.evaluation, frozen) {
        (CandidateEvaluation::Passed { .. }, Some(proof)) => validate_saved_proof(
            proof,
            Some(&proof.evaluation),
            crate::AttemptRole::PromotionFrozen,
            proof.candidate_digest,
            stage,
            session,
        )?,
        (CandidateEvaluation::Passed { .. }, None) => {
            return Err(invalid("a passing initial proof requires a frozen proof"));
        }
        (
            CandidateEvaluation::HardGateFailed { .. } | CandidateEvaluation::Quarantined { .. },
            None,
        ) => {}
        _ => {
            return Err(invalid(
                "the frozen proof does not match the initial terminal result",
            ));
        }
    }
    if frozen.is_some_and(|proof| proof.trial_id == baseline.trial_id) {
        return Err(invalid("the promotion trial identity is repeated"));
    }
    build_promotion_closure(stage, session, baseline, frozen)
}

fn build_promotion_closure(
    stage: &SearchStage,
    session: &SessionIdentity,
    baseline: &AuthenticatedEvaluationProof,
    frozen: Option<&AuthenticatedEvaluationProof>,
) -> Result<PromotionClosure, TuneError> {
    let policy_digest = crate::promotion_policy_digest(&stage.promotion)?;
    let baseline_anchor = Some(proof_anchor(baseline));
    let frozen_anchor = frozen.map(proof_anchor);
    if let Some(gate_id) = [Some(baseline), frozen]
        .into_iter()
        .flatten()
        .find_map(gate_failure)
    {
        return PromotionClosure::new(
            policy_digest,
            baseline_anchor,
            frozen_anchor,
            None,
            PromotionSelection {
                decision: PromotionDecision::RejectedHardGate { gate_id },
                selected_candidate: None,
            },
        );
    }
    if let Some(reason) = indeterminate_reason(baseline, frozen) {
        return PromotionClosure::new(
            policy_digest,
            baseline_anchor,
            frozen_anchor,
            None,
            PromotionSelection {
                decision: PromotionDecision::Indeterminate { reason },
                selected_candidate: None,
            },
        );
    }
    let frozen = frozen.ok_or_else(|| invalid("promotion has no frozen proof"))?;
    let calculation = crate::engine::calculate_promotion(
        stage,
        PromotionRunPlan {
            tuning_session_digest: session.digest()?,
            baseline_trial_id: baseline.trial_id,
            frozen_trial_id: frozen.trial_id,
            initial_candidate_digest: session.initial_candidate_digest,
            frozen_candidate_digest: frozen.candidate_digest,
            fixed_seed: session.fixed_seed,
        },
        &baseline.terminal_receipts,
        &frozen.terminal_receipts,
    )?;
    PromotionClosure::new(
        policy_digest,
        baseline_anchor,
        frozen_anchor,
        Some(calculation.comparison),
        calculation.selection,
    )
}

pub(super) fn promotion(
    state: &mut JournalState,
    closure: &PromotionClosure,
    stage: &SearchStage,
    session: &SessionIdentity,
) -> Result<(), TuneError> {
    closure.validate_for(&stage.promotion)?;
    let expected = expected_promotion_closure(state, stage, session)?;
    if closure != &expected {
        return Err(invalid(
            "the promotion closure does not match authenticated hidden results",
        ));
    }
    state.phase = CampaignPhase::PromotionClosed;
    state.promotion_decision = Some(closure.decision.clone());
    state.promotion_closure = Some(closure.clone());
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn seal(
    state: &mut JournalState,
    candidate: Digest,
    outcome: &FinalQualificationOutcome,
    promotion_closure_digest: Digest,
    final_evaluation_digest: Digest,
    final_proof_digest: Digest,
    initial: Digest,
    stage: &SearchStage,
) -> Result<(), TuneError> {
    let closure = state
        .promotion_closure
        .as_ref()
        .ok_or_else(|| invalid("the final seal has no promotion closure"))?;
    let proof = state
        .final_proof
        .as_ref()
        .ok_or_else(|| invalid("the final seal has no authenticated proof"))?;
    let authorized = state.authorized_final_candidate(initial)?;
    if state.phase != CampaignPhase::PromotionClosed
        || state.pending.is_some()
        || candidate != authorized
        || promotion_closure_digest != closure.closure_digest
        || final_evaluation_digest != proof.evaluation_digest
        || final_proof_digest != proof.proof_digest
        || !final_shape_matches(state.final_evaluation.as_ref(), outcome, stage)
    {
        return Err(invalid("the final seal does not match qualification"));
    }
    state.phase = CampaignPhase::Sealed;
    state.final_outcome = Some(outcome.clone());
    Ok(())
}

fn validate_optional_frozen_proof(
    state: &JournalState,
    proof: Option<&AuthenticatedEvaluationProof>,
    stage: &SearchStage,
    session: &SessionIdentity,
) -> Result<(), TuneError> {
    match (&state.promotion_baseline, proof, &state.promotion_frozen) {
        (Some(CandidateEvaluation::Passed { .. }), Some(proof), Some(evaluation)) => {
            let candidate = state
                .frozen_candidate
                .ok_or_else(|| invalid("promotion has no frozen candidate"))?;
            validate_saved_proof(
                proof,
                Some(evaluation),
                crate::AttemptRole::PromotionFrozen,
                candidate,
                stage,
                session,
            )
        }
        (Some(CandidateEvaluation::Passed { .. }), None, None) => {
            Err(invalid("a passing initial proof requires a frozen proof"))
        }
        (
            Some(
                CandidateEvaluation::HardGateFailed { .. }
                | CandidateEvaluation::Quarantined { .. },
            ),
            None,
            None,
        ) => Ok(()),
        _ => Err(invalid(
            "the frozen proof does not match the initial terminal result",
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_saved_proof(
    proof: &AuthenticatedEvaluationProof,
    evaluation: Option<&CandidateEvaluation>,
    role: crate::AttemptRole,
    candidate: Digest,
    stage: &SearchStage,
    session: &SessionIdentity,
) -> Result<(), TuneError> {
    match evaluation {
        Some(evaluation) => {
            proof.validate()?;
            let expected_plan = role.plan_digest(stage, candidate, session.fixed_seed)?;
            super::plan::validate_evaluation(&proof.evaluation, role, stage, session.fixed_seed)?;
            let session_digest = session.digest()?;
            if proof.role != role
                || proof.candidate_digest != candidate
                || proof.plan_digest != expected_plan
                || &proof.evaluation != evaluation
                || proof
                    .terminal_receipts
                    .iter()
                    .any(|receipt| receipt.context().tuning_session_digest() != session_digest)
            {
                return Err(invalid(
                    "a saved promotion proof changed its evaluation plan",
                ));
            }
            Ok(())
        }
        None => Err(invalid("a promotion proof has no saved evaluation")),
    }
}

fn proof_anchor(proof: &AuthenticatedEvaluationProof) -> (Digest, Digest) {
    (proof.evaluation_digest, proof.proof_digest)
}

fn gate_failure(proof: &AuthenticatedEvaluationProof) -> Option<String> {
    if let CandidateEvaluation::HardGateFailed { failure, .. } = &proof.evaluation {
        Some(failure.gate.id.clone())
    } else {
        None
    }
}

fn indeterminate_reason(
    baseline: &AuthenticatedEvaluationProof,
    frozen: Option<&AuthenticatedEvaluationProof>,
) -> Option<String> {
    for (name, proof) in [("initial", Some(baseline)), ("frozen", frozen)] {
        match proof.map(|value| &value.evaluation) {
            Some(CandidateEvaluation::Quarantined { reason }) => {
                return Some(format!(
                    "promotion {name} evaluation was quarantined: {reason}"
                ));
            }
            None => {}
            Some(
                CandidateEvaluation::Passed { .. } | CandidateEvaluation::HardGateFailed { .. },
            ) => {}
        }
    }
    None
}

fn final_shape_matches(
    evaluation: Option<&CandidateEvaluation>,
    outcome: &FinalQualificationOutcome,
    stage: &SearchStage,
) -> bool {
    crate::engine::final_outcome(stage, evaluation) == *outcome
}

fn invalid(detail: impl Into<String>) -> TuneError {
    TuneError::InvalidJournal {
        detail: detail.into(),
    }
}
