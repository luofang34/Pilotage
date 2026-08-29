use std::collections::BTreeMap;

use flight_tune::{
    AttemptRole, AuthenticatedEvaluationProof, CandidateEvaluation, Digest, JournalEvent,
    PromotionClosure, PromotionComparison, PromotionDecision, PromotionObjectiveResult, RunRecord,
    RunTerminalReceipt, SearchStage,
};
use serde::Serialize;

use crate::{CampaignEvidence, digest};

use super::super::super::statistics;

const EVALUATION_DOMAIN: &[u8] = b"pilotage.flight-tune.authenticated-evaluation.v1\0";
const PROOF_DOMAIN: &[u8] = b"pilotage.flight-tune.authenticated-evaluation-proof.v1\0";
const POLICY_DOMAIN: &[u8] = b"pilotage.flight-tune.promotion-policy.v1\0";
const COMPARISON_DOMAIN: &[u8] = b"pilotage.flight-tune.promotion-comparison.v1\0";
const DECISION_DOMAIN: &[u8] = b"pilotage.flight-tune.promotion-decision.v1\0";
const SELECTION_DOMAIN: &[u8] = b"pilotage.flight-tune.promotion-selection.v1\0";
const CLOSURE_DOMAIN: &[u8] = b"pilotage.flight-tune.promotion-closure.v1\0";

#[derive(Serialize)]
struct EvaluationDocument<'a> {
    schema_version: u16,
    trial_id: u64,
    role: AttemptRole,
    candidate_digest: Digest,
    plan_digest: Digest,
    retry_index: u32,
    evaluation: &'a CandidateEvaluation,
}

#[derive(Serialize)]
struct ProofDocument<'a> {
    schema_version: u16,
    evaluation_digest: Digest,
    receipt_digests: &'a [Digest],
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

pub(super) fn promotion_closure(
    stage: &SearchStage,
    baseline: &AuthenticatedEvaluationProof,
    frozen: &AuthenticatedEvaluationProof,
) -> PromotionClosure {
    let comparison = promotion_comparison(stage, baseline, frozen);
    let promoted = comparison.loss_passed
        && comparison.control_effort_passed
        && comparison.objectives.values().all(|result| result.passed);
    let mut closure = PromotionClosure {
        schema_version: flight_tune::PROMOTION_CLOSURE_SCHEMA_VERSION,
        policy_digest: digest::domain("promotion policy", POLICY_DOMAIN, &stage.promotion)
            .expect("policy digest"),
        baseline_evaluation_digest: Some(baseline.evaluation_digest),
        baseline_proof_digest: Some(baseline.proof_digest),
        frozen_evaluation_digest: Some(frozen.evaluation_digest),
        frozen_proof_digest: Some(frozen.proof_digest),
        comparison: Some(comparison),
        comparison_digest: None,
        decision: if promoted {
            PromotionDecision::Promoted {}
        } else {
            PromotionDecision::RejectedNoImprovement {}
        },
        decision_digest: Digest::from_bytes([0; 32]),
        selected_candidate: Some(if promoted {
            frozen.candidate_digest
        } else {
            baseline.candidate_digest
        }),
        selection_digest: Digest::from_bytes([0; 32]),
        closure_digest: Digest::from_bytes([0; 32]),
    };
    refresh_closure(&mut closure);
    closure
}

fn promotion_comparison(
    stage: &SearchStage,
    baseline: &AuthenticatedEvaluationProof,
    frozen: &AuthenticatedEvaluationProof,
) -> PromotionComparison {
    let baseline_runs = passed_runs(baseline);
    let frozen_runs = passed_runs(frozen);
    let baseline_mean_loss =
        statistics::mean(baseline_runs.iter().map(|run| run.loss)).expect("baseline mean");
    let loss = statistics::paired(
        baseline_runs
            .iter()
            .zip(frozen_runs)
            .map(|(left, right)| right.loss - left.loss),
    )
    .expect("loss statistics");
    let control_effort = statistics::paired(
        baseline_runs
            .iter()
            .zip(frozen_runs)
            .map(|(left, right)| right.control_effort - left.control_effort),
    )
    .expect("effort statistics");
    let objective =
        statistics::paired(baseline_runs.iter().zip(frozen_runs).map(|(left, right)| {
            right.objectives.get("tracking").expect("frozen objective")
                - left.objectives.get("tracking").expect("baseline objective")
        }))
        .expect("objective statistics");
    let maximum_objective = *stage
        .promotion
        .objective_regression_upper_95
        .get("tracking")
        .expect("objective limit");
    let required = stage
        .promotion
        .minimum_loss_improvement
        .max(baseline_mean_loss * stage.promotion.minimum_relative_loss_improvement);
    PromotionComparison {
        baseline_mean_loss,
        required_loss_improvement: required,
        loss,
        loss_passed: loss.upper_95 <= -required,
        control_effort,
        control_effort_passed: control_effort.mean
            <= stage.promotion.maximum_control_effort_increase,
        objectives: BTreeMap::from([(
            "tracking".to_owned(),
            PromotionObjectiveResult {
                statistics: objective,
                maximum_upper_95: maximum_objective,
                passed: objective.upper_95 <= maximum_objective,
            },
        )]),
    }
}

fn passed_runs(proof: &AuthenticatedEvaluationProof) -> &[RunRecord] {
    let CandidateEvaluation::Passed { runs, .. } = &proof.evaluation else {
        panic!("fixture evaluation must pass");
    };
    runs
}

pub(super) fn refresh_proof(proof: &mut AuthenticatedEvaluationProof) {
    proof.evaluation_digest = digest::domain(
        "authenticated evaluation",
        EVALUATION_DOMAIN,
        &EvaluationDocument {
            schema_version: proof.schema_version,
            trial_id: proof.trial_id,
            role: proof.role,
            candidate_digest: proof.candidate_digest,
            plan_digest: proof.plan_digest,
            retry_index: proof.retry_index,
            evaluation: &proof.evaluation,
        },
    )
    .expect("evaluation digest");
    let receipt_digests = proof
        .terminal_receipts
        .iter()
        .map(RunTerminalReceipt::receipt_digest)
        .collect::<Vec<_>>();
    proof.proof_digest = digest::domain(
        "authenticated evaluation proof",
        PROOF_DOMAIN,
        &ProofDocument {
            schema_version: proof.schema_version,
            evaluation_digest: proof.evaluation_digest,
            receipt_digests: &receipt_digests,
        },
    )
    .expect("proof digest");
}

pub(super) fn refresh_closure(closure: &mut PromotionClosure) {
    closure.comparison_digest = closure.comparison.as_ref().map(|comparison| {
        digest::domain("promotion comparison", COMPARISON_DOMAIN, comparison)
            .expect("comparison digest")
    });
    closure.decision_digest =
        digest::domain("promotion decision", DECISION_DOMAIN, &closure.decision)
            .expect("decision digest");
    closure.selection_digest = digest::domain(
        "promotion selection",
        SELECTION_DOMAIN,
        &SelectionDocument {
            decision: &closure.decision,
            decision_digest: closure.decision_digest,
            selected_candidate: closure.selected_candidate,
        },
    )
    .expect("selection digest");
    closure.closure_digest = digest::domain(
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
    )
    .expect("closure digest");
}

pub(super) fn refresh_head(evidence: &mut CampaignEvidence) {
    let proof = evidence.journal.final_proof.as_ref().expect("final proof");
    let JournalEvent::Sealed {
        final_evaluation_digest,
        final_proof_digest,
        ..
    } = &mut evidence.journal.head.entry.event
    else {
        panic!("fixture head must be sealed");
    };
    *final_evaluation_digest = proof.evaluation_digest;
    *final_proof_digest = proof.proof_digest;
    evidence.journal.head.entry_digest =
        digest::document("journal entry", &evidence.journal.head.entry).expect("head digest");
}
