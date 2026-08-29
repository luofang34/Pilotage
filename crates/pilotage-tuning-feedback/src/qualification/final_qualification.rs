use flight_tune::{
    CandidateEvaluation, FinalQualificationOutcome, JournalEvent, JournalEvidenceSnapshot,
    PromotionDecision, SessionIdentity,
};

use crate::{FeedbackError, error::invalid};

use super::campaign::CampaignIdentity;
use super::{evaluation, promotion::PromotionResult};

pub(super) struct FinalResult {
    pub(super) outcome: Option<FinalQualificationOutcome>,
}

pub(super) fn verify(
    snapshot: &JournalEvidenceSnapshot,
    session: &SessionIdentity,
    identity: &CampaignIdentity,
    promotion: &PromotionResult,
) -> Result<FinalResult, FeedbackError> {
    match &snapshot.head.entry.event {
        JournalEvent::PromotionClosed { closure } => {
            if closure != &snapshot.promotion_closure
                || snapshot.final_proof.is_some()
                || snapshot.final_outcome.is_some()
                || identity.authority.final_qualification.is_some()
            {
                return Err(invalid("an open final head contains sealed evidence"));
            }
            Ok(FinalResult { outcome: None })
        }
        JournalEvent::Sealed {
            candidate,
            outcome,
            promotion_closure_digest,
            final_evaluation_digest,
            final_proof_digest,
        } => verify_sealed(
            snapshot,
            session,
            identity,
            promotion,
            *candidate,
            outcome,
            *promotion_closure_digest,
            *final_evaluation_digest,
            *final_proof_digest,
        ),
        _ => Err(invalid("the journal head is not a stable evidence head")),
    }
}

#[allow(clippy::too_many_arguments)]
fn verify_sealed(
    snapshot: &JournalEvidenceSnapshot,
    session: &SessionIdentity,
    identity: &CampaignIdentity,
    promotion: &PromotionResult,
    candidate: flight_tune::Digest,
    outcome: &FinalQualificationOutcome,
    promotion_closure_digest: flight_tune::Digest,
    final_evaluation_digest: flight_tune::Digest,
    final_proof_digest: flight_tune::Digest,
) -> Result<FinalResult, FeedbackError> {
    let selected = promotion
        .selected_candidate
        .ok_or_else(|| invalid("a rejected promotion result cannot enter final qualification"))?;
    if !matches!(
        snapshot.promotion_closure.decision,
        PromotionDecision::Promoted {} | PromotionDecision::RejectedNoImprovement {}
    ) || candidate != selected
        || identity.authority.final_candidate != Some(selected)
    {
        return Err(invalid(
            "the sealed candidate is not authorized by promotion",
        ));
    }
    let proof = snapshot
        .final_proof
        .as_ref()
        .ok_or_else(|| invalid("a sealed head has no final proof"))?;
    let final_attempt = identity
        .authority
        .final_qualification
        .ok_or_else(|| invalid("a sealed head has no final attempt authority"))?;
    if final_attempt.trial_id == identity.authority.promotion_baseline.trial_id
        || identity
            .authority
            .promotion_frozen
            .is_some_and(|promotion| promotion.trial_id == final_attempt.trial_id)
    {
        return Err(invalid(
            "the final trial identity repeats a promotion trial",
        ));
    }
    let verified = evaluation::verify(
        proof,
        &snapshot.stage,
        session,
        identity.session_digest,
        final_attempt,
    )?;
    let expected_outcome = outcome_for(&snapshot.stage, &verified.proof.evaluation)?;
    if snapshot.final_outcome.as_ref() != Some(outcome)
        || outcome != &expected_outcome
        || promotion_closure_digest != snapshot.promotion_closure.closure_digest
        || final_evaluation_digest != proof.evaluation_digest
        || final_proof_digest != proof.proof_digest
    {
        return Err(invalid("the sealed final qualification result changed"));
    }
    Ok(FinalResult {
        outcome: Some(expected_outcome),
    })
}

fn outcome_for(
    stage: &flight_tune::SearchStage,
    evaluation: &CandidateEvaluation,
) -> Result<FinalQualificationOutcome, FeedbackError> {
    match evaluation {
        CandidateEvaluation::Passed { aggregate, runs } => {
            let scalar = [
                (
                    "loss_confidence_95.upper",
                    aggregate.loss_confidence_95.upper,
                    stage.qualification.maximum_loss_confidence_upper,
                ),
                (
                    "p95_loss",
                    aggregate.p95_loss,
                    stage.qualification.maximum_p95_loss,
                ),
                (
                    "mean_control_effort",
                    aggregate.mean_control_effort,
                    stage.qualification.maximum_mean_control_effort,
                ),
            ]
            .into_iter()
            .find(|(_, actual, maximum)| actual > maximum);
            if let Some((metric, _, _)) = scalar {
                return Ok(FinalQualificationOutcome::FailedObjective {
                    metric: metric.to_owned(),
                });
            }
            // The authority band is absolute and per run. A candidate that
            // improved every normalized metric by resolving less physical
            // speed for the same operator input did not improve the command
            // law, and no other measurement here can see that.
            for run in runs {
                if !super::response_target::authority_holds(stage, run) {
                    return Ok(FinalQualificationOutcome::FailedObjective {
                        metric: flight_tune::TARGET_AUTHORITY_OBJECTIVE.to_owned(),
                    });
                }
            }
            for metric in &stage.qualification.objectives {
                let mut failed = false;
                for run in runs {
                    let target =
                        super::response_target::row(stage, &run.mission_revision_id, metric)?;
                    let value =
                        run.objectives.get(metric).copied().ok_or_else(|| {
                            invalid(format!("a final run has no objective {metric}"))
                        })?;
                    failed |= !super::response_target::holds(target, value);
                }
                if failed {
                    return Ok(FinalQualificationOutcome::FailedObjective {
                        metric: metric.clone(),
                    });
                }
            }
            Ok(FinalQualificationOutcome::Qualified)
        }
        CandidateEvaluation::HardGateFailed { failure, .. } => {
            Ok(FinalQualificationOutcome::FailedHardGate {
                gate_id: failure.gate.id.clone(),
            })
        }
        CandidateEvaluation::Quarantined { reason } => {
            Ok(FinalQualificationOutcome::Indeterminate {
                reason: reason.clone(),
            })
        }
    }
}
