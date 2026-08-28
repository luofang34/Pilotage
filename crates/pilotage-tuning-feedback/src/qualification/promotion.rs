use std::collections::BTreeMap;

use flight_tune::{
    CandidateEvaluation, Digest, JournalEvidenceSnapshot, PromotionClosure, PromotionComparison,
    PromotionDecision, PromotionObjectiveResult, RunRecord, SearchStage, SessionIdentity,
};
use serde::Serialize;

use crate::{FeedbackError, digest, error::invalid};

use super::campaign::CampaignIdentity;
use super::evaluation;
use super::statistics;

const POLICY_DOMAIN: &[u8] = b"pilotage.flight-tune.promotion-policy.v1\0";
const COMPARISON_DOMAIN: &[u8] = b"pilotage.flight-tune.promotion-comparison.v1\0";
const DECISION_DOMAIN: &[u8] = b"pilotage.flight-tune.promotion-decision.v1\0";
const SELECTION_DOMAIN: &[u8] = b"pilotage.flight-tune.promotion-selection.v1\0";
const CLOSURE_DOMAIN: &[u8] = b"pilotage.flight-tune.promotion-closure.v1\0";
const PROMOTION_CLOSURE_SCHEMA_VERSION: u16 = 1;

pub(super) struct PromotionResult {
    pub(super) selected_candidate: Option<Digest>,
}

struct ExpectedClosure {
    comparison: Option<PromotionComparison>,
    decision: PromotionDecision,
    selected_candidate: Option<Digest>,
}

#[derive(Serialize)]
struct SelectionDocument<'a> {
    decision: &'a PromotionDecision,
    decision_digest: Digest,
    selected_candidate: Option<Digest>,
}

#[derive(Serialize)]
struct ClosureDocument<'a> {
    schema_version: u16,
    policy_digest: Digest,
    baseline_evaluation_digest: Option<Digest>,
    baseline_proof_digest: Option<Digest>,
    frozen_evaluation_digest: Option<Digest>,
    frozen_proof_digest: Option<Digest>,
    comparison: &'a Option<PromotionComparison>,
    comparison_digest: Option<Digest>,
    decision: &'a PromotionDecision,
    decision_digest: Digest,
    selected_candidate: Option<Digest>,
    selection_digest: Digest,
}

pub(super) fn verify(
    snapshot: &JournalEvidenceSnapshot,
    session: &SessionIdentity,
    identity: &CampaignIdentity,
) -> Result<PromotionResult, FeedbackError> {
    let baseline = evaluation::verify(
        &snapshot.promotion_baseline,
        &snapshot.stage,
        session,
        identity.session_digest,
        identity.authority.promotion_baseline,
    )?;
    let frozen = verify_frozen(snapshot, session, identity, &baseline)?;
    if frozen
        .as_ref()
        .is_some_and(|proof| proof.proof.trial_id == baseline.proof.trial_id)
    {
        return Err(invalid("the promotion trial identity is repeated"));
    }
    let expected = expected_closure(
        &snapshot.stage,
        identity.authority.baseline_candidate,
        identity.authority.frozen_candidate,
        &baseline,
        frozen.as_ref(),
    )?;
    verify_closure(
        &snapshot.promotion_closure,
        &snapshot.stage,
        &baseline,
        frozen.as_ref(),
        &expected,
    )?;
    if identity.authority.final_candidate != expected.selected_candidate {
        return Err(invalid("the final candidate authority changed"));
    }
    Ok(PromotionResult {
        selected_candidate: expected.selected_candidate,
    })
}

fn verify_frozen<'a>(
    snapshot: &'a JournalEvidenceSnapshot,
    session: &SessionIdentity,
    identity: &CampaignIdentity,
    baseline: &evaluation::VerifiedProof<'_>,
) -> Result<Option<evaluation::VerifiedProof<'a>>, FeedbackError> {
    match (
        &baseline.proof.evaluation,
        &snapshot.promotion_frozen,
        identity.authority.promotion_frozen,
    ) {
        (CandidateEvaluation::Passed { .. }, Some(proof), Some(attempt)) => evaluation::verify(
            proof,
            &snapshot.stage,
            session,
            identity.session_digest,
            attempt,
        )
        .map(Some),
        (CandidateEvaluation::Passed { .. }, None, None) => {
            Err(invalid("a passing promotion baseline has no frozen proof"))
        }
        (
            CandidateEvaluation::HardGateFailed { .. } | CandidateEvaluation::Quarantined { .. },
            None,
            None,
        ) => Ok(None),
        _ => Err(invalid(
            "the frozen promotion proof does not match the baseline result",
        )),
    }
}

fn expected_closure(
    stage: &SearchStage,
    initial_candidate: Digest,
    frozen_candidate: Digest,
    baseline: &evaluation::VerifiedProof<'_>,
    frozen: Option<&evaluation::VerifiedProof<'_>>,
) -> Result<ExpectedClosure, FeedbackError> {
    if let Some(gate_id) = [Some(baseline), frozen]
        .into_iter()
        .flatten()
        .find_map(gate_failure)
    {
        return Ok(ExpectedClosure {
            comparison: None,
            decision: PromotionDecision::RejectedHardGate { gate_id },
            selected_candidate: None,
        });
    }
    if let Some(reason) = indeterminate_reason(baseline, frozen) {
        return Ok(ExpectedClosure {
            comparison: None,
            decision: PromotionDecision::Indeterminate { reason },
            selected_candidate: None,
        });
    }
    let frozen = frozen.ok_or_else(|| invalid("promotion has no frozen proof"))?;
    let comparison = compare(stage, baseline, frozen)?;
    let passed = comparison_passes(&comparison);
    Ok(ExpectedClosure {
        decision: if passed {
            PromotionDecision::Promoted {}
        } else {
            PromotionDecision::RejectedNoImprovement {}
        },
        selected_candidate: Some(if passed {
            frozen_candidate
        } else {
            initial_candidate
        }),
        comparison: Some(comparison),
    })
}

fn compare(
    stage: &SearchStage,
    baseline: &evaluation::VerifiedProof<'_>,
    frozen: &evaluation::VerifiedProof<'_>,
) -> Result<PromotionComparison, FeedbackError> {
    let baseline_runs = passing_runs(baseline)?;
    let frozen_runs = passing_runs(frozen)?;
    if baseline_runs.len() != frozen_runs.len() || baseline_runs.len() < 2 {
        return Err(invalid("promotion run keys do not form exact pairs"));
    }
    let policy = &stage.promotion;
    let baseline_mean_loss = statistics::mean(baseline_runs.iter().map(|run| run.loss))?;
    let loss = statistics::paired(
        baseline_runs
            .iter()
            .zip(frozen_runs)
            .map(|(left, right)| right.loss - left.loss),
    )?;
    let control_effort = statistics::paired(
        baseline_runs
            .iter()
            .zip(frozen_runs)
            .map(|(left, right)| right.control_effort - left.control_effort),
    )?;
    let required_loss_improvement = policy
        .minimum_loss_improvement
        .max(baseline_mean_loss * policy.minimum_relative_loss_improvement);
    if !required_loss_improvement.is_finite() {
        return Err(invalid("promotion threshold arithmetic is not finite"));
    }
    Ok(PromotionComparison {
        baseline_mean_loss,
        required_loss_improvement,
        loss,
        loss_passed: within_upper_limit(loss.upper_95, -required_loss_improvement),
        control_effort,
        control_effort_passed: within_upper_limit(
            control_effort.mean,
            policy.maximum_control_effort_increase,
        ),
        objectives: objective_results(stage, baseline_runs, frozen_runs)?,
    })
}

fn objective_results(
    stage: &SearchStage,
    baseline: &[RunRecord],
    frozen: &[RunRecord],
) -> Result<BTreeMap<String, PromotionObjectiveResult>, FeedbackError> {
    let mut results = BTreeMap::new();
    for (name, maximum) in &stage.promotion.objective_regression_upper_95 {
        let mut deltas = Vec::with_capacity(baseline.len());
        for (left, right) in baseline.iter().zip(frozen) {
            let left = left
                .objectives
                .get(name)
                .copied()
                .ok_or_else(|| invalid(format!("a baseline run has no objective {name}")))?;
            let right = right
                .objectives
                .get(name)
                .copied()
                .ok_or_else(|| invalid(format!("a frozen run has no objective {name}")))?;
            deltas.push(right - left);
        }
        let statistics = statistics::paired(deltas.into_iter())?;
        results.insert(
            name.clone(),
            PromotionObjectiveResult {
                statistics,
                maximum_upper_95: *maximum,
                passed: within_upper_limit(statistics.upper_95, *maximum),
            },
        );
    }
    Ok(results)
}

fn verify_closure(
    closure: &PromotionClosure,
    stage: &SearchStage,
    baseline: &evaluation::VerifiedProof<'_>,
    frozen: Option<&evaluation::VerifiedProof<'_>>,
    expected: &ExpectedClosure,
) -> Result<(), FeedbackError> {
    let policy_digest = digest::domain("promotion policy", POLICY_DOMAIN, &stage.promotion)?;
    let frozen_anchor =
        frozen.map(|proof| (proof.proof.evaluation_digest, proof.proof.proof_digest));
    if closure.schema_version != PROMOTION_CLOSURE_SCHEMA_VERSION
        || closure.policy_digest != policy_digest
        || closure.baseline_evaluation_digest != Some(baseline.proof.evaluation_digest)
        || closure.baseline_proof_digest != Some(baseline.proof.proof_digest)
        || closure.frozen_evaluation_digest != frozen_anchor.map(|anchor| anchor.0)
        || closure.frozen_proof_digest != frozen_anchor.map(|anchor| anchor.1)
        || closure.comparison != expected.comparison
        || closure.decision != expected.decision
        || closure.selected_candidate != expected.selected_candidate
    {
        return Err(invalid("the promotion closure result changed"));
    }
    verify_closure_digests(closure)
}

fn verify_closure_digests(closure: &PromotionClosure) -> Result<(), FeedbackError> {
    let comparison_digest = closure
        .comparison
        .as_ref()
        .map(|value| digest::domain("promotion comparison", COMPARISON_DOMAIN, value))
        .transpose()?;
    let decision_digest = digest::domain("promotion decision", DECISION_DOMAIN, &closure.decision)?;
    let selection_digest = digest::domain(
        "promotion selection",
        SELECTION_DOMAIN,
        &SelectionDocument {
            decision: &closure.decision,
            decision_digest,
            selected_candidate: closure.selected_candidate,
        },
    )?;
    let closure_digest = digest::domain(
        "promotion closure",
        CLOSURE_DOMAIN,
        &ClosureDocument {
            schema_version: closure.schema_version,
            policy_digest: closure.policy_digest,
            baseline_evaluation_digest: closure.baseline_evaluation_digest,
            baseline_proof_digest: closure.baseline_proof_digest,
            frozen_evaluation_digest: closure.frozen_evaluation_digest,
            frozen_proof_digest: closure.frozen_proof_digest,
            comparison: &closure.comparison,
            comparison_digest: closure.comparison_digest,
            decision: &closure.decision,
            decision_digest: closure.decision_digest,
            selected_candidate: closure.selected_candidate,
            selection_digest: closure.selection_digest,
        },
    )?;
    if closure.comparison_digest != comparison_digest
        || closure.decision_digest != decision_digest
        || closure.selection_digest != selection_digest
        || closure.closure_digest != closure_digest
        || closure.closure_digest.is_zero()
    {
        return Err(invalid("a promotion closure digest changed"));
    }
    Ok(())
}

fn gate_failure(proof: &evaluation::VerifiedProof<'_>) -> Option<String> {
    if let CandidateEvaluation::HardGateFailed { failure, .. } = &proof.proof.evaluation {
        Some(failure.gate.id.clone())
    } else {
        None
    }
}

fn indeterminate_reason(
    baseline: &evaluation::VerifiedProof<'_>,
    frozen: Option<&evaluation::VerifiedProof<'_>>,
) -> Option<String> {
    for (name, proof) in [("initial", Some(baseline)), ("frozen", frozen)] {
        if let Some(CandidateEvaluation::Quarantined { reason }) =
            proof.map(|value| &value.proof.evaluation)
        {
            return Some(format!(
                "promotion {name} evaluation was quarantined: {reason}"
            ));
        }
    }
    None
}

fn passing_runs<'a>(
    proof: &'a evaluation::VerifiedProof<'_>,
) -> Result<&'a [RunRecord], FeedbackError> {
    if let CandidateEvaluation::Passed { runs, .. } = &proof.proof.evaluation {
        Ok(runs)
    } else {
        Err(invalid("promotion expected a passing evaluation"))
    }
}

fn comparison_passes(comparison: &PromotionComparison) -> bool {
    comparison.loss_passed
        && comparison.control_effort_passed
        && comparison.objectives.values().all(|result| result.passed)
}

const fn within_upper_limit(actual: f64, maximum: f64) -> bool {
    actual <= maximum
}

#[cfg(test)]
mod tests {
    use super::within_upper_limit;

    #[test]
    fn an_upper_limit_accepts_equality_and_rejects_next_up() {
        let maximum = 0.05_f64;
        let next = f64::from_bits(maximum.to_bits().wrapping_add(1));
        assert!(within_upper_limit(maximum, maximum));
        assert!(!within_upper_limit(next, maximum));
    }
}
