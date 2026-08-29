#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use flight_tune::{
    ArtifactIdentity, AttemptRole, AuthenticatedEvaluationProof, Candidate, CandidateEvaluation,
    CandidateLineage, CandidateTransitionReference, Digest, JournalEvent, MissionReference,
    ParameterBounds, PromotionPolicy, PromotionSeedPolicy, QualificationPolicy, RuntimeIdentities,
    SearchStage, SessionIdentity,
};

use crate::CampaignEvidence;
use crate::digest;

use super::super::{plan, statistics};

mod attempts;
mod authority;
mod quarantine;
mod sealing;
mod terminal;

use authority::sealed_campaign;
pub(super) use quarantine::quarantined_proof;
use sealing::promotion_closure;
use terminal::{receipt, run};

#[derive(Clone, Copy)]
struct Point {
    loss: f64,
    effort: f64,
    objective: f64,
}

pub(super) fn fixture() -> CampaignEvidence {
    let stage = stage();
    let session = session(&stage);
    let session_digest = digest::document("session identity", &session).expect("session digest");
    let baseline = proof(
        &stage,
        &session,
        session_digest,
        2,
        AttemptRole::PromotionBaseline,
        session.initial_candidate_digest,
        Point {
            loss: 1.0,
            effort: 0.30,
            objective: 0.20,
        },
    );
    let frozen_candidate = candidate_digest(0.5);
    let frozen = proof(
        &stage,
        &session,
        session_digest,
        3,
        AttemptRole::PromotionFrozen,
        frozen_candidate,
        Point {
            loss: 0.80,
            effort: 0.35,
            objective: 0.21,
        },
    );
    let closure = promotion_closure(&stage, &baseline, &frozen);
    let final_proof = proof(
        &stage,
        &session,
        session_digest,
        4,
        AttemptRole::FinalQualification,
        frozen_candidate,
        Point {
            loss: 0.50,
            effort: 0.20,
            objective: 0.10,
        },
    );
    sealed_campaign(
        stage,
        session,
        baseline,
        frozen,
        closure,
        final_proof,
        frozen_candidate,
    )
}

fn stage() -> SearchStage {
    SearchStage {
        execution_retry: flight_tune::ExecutionRetryPolicy::none(),
        id: "golden-stage".to_owned(),
        allowlist: BTreeMap::from([(
            "rate".to_owned(),
            ParameterBounds {
                minimum: 0.0,
                maximum: 1.0,
            },
        )]),
        fixed_parameters: BTreeMap::new(),
        required_hard_gates: vec!["crash".to_owned(), "finite".to_owned()],
        training_scenarios: vec![scenario("training-calm", 11)],
        promotion_scenarios: vec![scenario("promotion-calm", 12)],
        final_qualification_scenarios: vec![
            scenario("final-calm", 13),
            scenario("final-crosswind", 14),
        ],
        repetitions: 3,
        promotion: PromotionPolicy {
            schema_version: flight_tune::PROMOTION_POLICY_SCHEMA_VERSION,
            seed_policy: PromotionSeedPolicy::PairedScenarioDigestV1,
            minimum_loss_improvement: 0.10,
            minimum_relative_loss_improvement: 0.05,
            maximum_control_effort_increase: 0.10,
            objective_regression_upper_95: BTreeMap::from([("tracking".to_owned(), 0.05)]),
        },
        qualification: QualificationPolicy {
            maximum_loss_confidence_upper: 1.0,
            maximum_p95_loss: 1.0,
            maximum_mean_control_effort: 1.0,
            objective_maxima: BTreeMap::from([("tracking".to_owned(), 1.0)]),
        },
    }
}

fn session(stage: &SearchStage) -> SessionIdentity {
    SessionIdentity {
        stage_digest: digest::document("search stage", stage).expect("stage digest"),
        initial_candidate_digest: candidate_digest(0.0),
        candidate_lineage: CandidateLineage {
            schema: "pilotage.test.candidate.v1".to_owned(),
            base_preset_digest: fixed_digest(2),
            plant_digest: fixed_digest(3),
        },
        fixed_seed: 23,
        runtimes: RuntimeIdentities {
            harness_build: artifact("harness", 31),
            strategy: artifact("strategy", 32),
            metric: artifact("metric", 33),
            hard_gates: artifact("gates", 34),
            scenario_runtime: None,
            simulator: artifact("simulator", 35),
            airframe: artifact("airframe", 36),
            vehicle: artifact("vehicle", 37),
            transition_validator: artifact("transition", 38),
            adjacency_policy_digest: fixed_digest(39),
        },
    }
}

fn proof(
    stage: &SearchStage,
    session: &SessionIdentity,
    session_digest: Digest,
    trial_id: u64,
    role: AttemptRole,
    candidate: Digest,
    point: Point,
) -> AuthenticatedEvaluationProof {
    proof_with_objectives(
        stage,
        session,
        session_digest,
        trial_id,
        role,
        candidate,
        point,
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn proof_with_objectives(
    stage: &SearchStage,
    session: &SessionIdentity,
    session_digest: Digest,
    trial_id: u64,
    role: AttemptRole,
    candidate: Digest,
    point: Point,
    missing_objective_at: Option<usize>,
    transition: Option<CandidateTransitionReference>,
) -> AuthenticatedEvaluationProof {
    let expected = plan::expected_runs(
        stage,
        role,
        candidate,
        trial_id,
        session.fixed_seed,
        session_digest,
        0,
    );
    let runs = expected
        .iter()
        .enumerate()
        .map(|(index, expected)| run(expected, point, missing_objective_at == Some(index)))
        .collect::<Vec<_>>();
    let receipts = expected
        .iter()
        .zip(&runs)
        .map(|(expected, run)| {
            receipt(expected, run.clone(), &session.runtimes.vehicle, transition)
        })
        .collect::<Vec<_>>();
    let evaluation = CandidateEvaluation::Passed {
        aggregate: statistics::aggregate(&runs).expect("aggregate proof runs"),
        runs,
    };
    let mut proof = AuthenticatedEvaluationProof {
        retry_index: 0,
        schema_version: flight_tune::AUTHENTICATED_EVALUATION_PROOF_SCHEMA_VERSION,
        trial_id,
        role,
        candidate_digest: candidate,
        plan_digest: plan::digest_for(stage, role, candidate, session.fixed_seed)
            .expect("run plan digest"),
        evaluation,
        terminal_receipts: receipts,
        evaluation_digest: Digest::from_bytes([0; 32]),
        proof_digest: Digest::from_bytes([0; 32]),
    };
    refresh_proof(&mut proof);
    proof
}

pub(super) fn proof_with_missing_objective(
    stage: &SearchStage,
    session: &SessionIdentity,
    trial_id: u64,
    role: AttemptRole,
    candidate: Digest,
) -> AuthenticatedEvaluationProof {
    let session_digest = digest::document("session identity", session).expect("session digest");
    proof_with_objectives(
        stage,
        session,
        session_digest,
        trial_id,
        role,
        candidate,
        Point {
            loss: 1.0,
            effort: 0.30,
            objective: 0.20,
        },
        Some(1),
        None,
    )
}

pub(super) fn proof_with_hard_gates(
    stage: &SearchStage,
    session: &SessionIdentity,
    trial_id: u64,
    role: AttemptRole,
    candidate: Digest,
    hard_gates: &[&str],
) -> AuthenticatedEvaluationProof {
    let session_digest = digest::document("session identity", session).expect("session digest");
    let mut proof = proof(
        stage,
        session,
        session_digest,
        trial_id,
        role,
        candidate,
        Point {
            loss: 1.0,
            effort: 0.30,
            objective: 0.20,
        },
    );
    let expected = plan::expected_runs(
        stage,
        role,
        candidate,
        trial_id,
        session.fixed_seed,
        session_digest,
        0,
    );
    let CandidateEvaluation::Passed { runs, .. } = &mut proof.evaluation else {
        panic!("fixture evaluation must pass");
    };
    runs[0].passed_hard_gates = hard_gates.iter().map(|gate| (*gate).to_owned()).collect();
    proof.terminal_receipts[0] = receipt(
        &expected[0],
        runs[0].clone(),
        &session.runtimes.vehicle,
        None,
    );
    refresh_proof(&mut proof);
    proof
}

pub(super) fn refresh_proof(proof: &mut AuthenticatedEvaluationProof) {
    sealing::refresh_proof(proof);
}

pub(super) fn refresh_head(evidence: &mut CampaignEvidence) {
    sealing::refresh_head(evidence);
    let proof = evidence
        .journal
        .final_proof
        .as_ref()
        .expect("fixture has a final proof")
        .clone();
    rewrite_hidden_attempt(evidence, AttemptRole::FinalQualification, &proof);
    let frozen_candidate = evidence.journal.authority.frozen_candidate;
    rewrite_promotion_authority(evidence, frozen_candidate);
    rechain_journal_authority(evidence);
}

pub(super) fn refresh_promotion_authority(evidence: &mut CampaignEvidence) {
    let frozen = evidence.journal.promotion_frozen.as_ref();
    evidence
        .journal
        .promotion_closure
        .baseline_evaluation_digest = Some(evidence.journal.promotion_baseline.evaluation_digest);
    evidence.journal.promotion_closure.baseline_proof_digest =
        Some(evidence.journal.promotion_baseline.proof_digest);
    evidence.journal.promotion_closure.frozen_evaluation_digest =
        frozen.map(|proof| proof.evaluation_digest);
    evidence.journal.promotion_closure.frozen_proof_digest = frozen.map(|proof| proof.proof_digest);
    sync_promotion_closure(evidence);
}

pub(super) fn rebuild_promotion_closure(evidence: &mut CampaignEvidence) {
    let frozen = evidence
        .journal
        .promotion_frozen
        .as_ref()
        .expect("fixture has a frozen promotion proof");
    evidence.journal.promotion_closure = promotion_closure(
        &evidence.journal.stage,
        &evidence.journal.promotion_baseline,
        frozen,
    );
    sync_promotion_closure(evidence);
}

fn sync_promotion_closure(evidence: &mut CampaignEvidence) {
    sealing::refresh_closure(&mut evidence.journal.promotion_closure);
    let JournalEvent::Sealed {
        promotion_closure_digest,
        ..
    } = &mut evidence.journal.head.entry.event
    else {
        panic!("fixture head must be sealed");
    };
    *promotion_closure_digest = evidence.journal.promotion_closure.closure_digest;
    sealing::refresh_head(evidence);
    let baseline = evidence.journal.promotion_baseline.clone();
    rewrite_hidden_attempt(evidence, AttemptRole::PromotionBaseline, &baseline);
    if let Some(frozen) = evidence.journal.promotion_frozen.clone() {
        rewrite_hidden_attempt(evidence, AttemptRole::PromotionFrozen, &frozen);
    }
    let frozen_candidate = evidence.journal.authority.frozen_candidate;
    rewrite_promotion_authority(evidence, frozen_candidate);
    rechain_journal_authority(evidence);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn passing_proof(
    stage: &SearchStage,
    session: &SessionIdentity,
    trial_id: u64,
    role: AttemptRole,
    candidate: Digest,
    loss: f64,
    effort: f64,
    objective: f64,
) -> AuthenticatedEvaluationProof {
    let session_digest = digest::document("session identity", session).expect("session digest");
    proof(
        stage,
        session,
        session_digest,
        trial_id,
        role,
        candidate,
        Point {
            loss,
            effort,
            objective,
        },
    )
}

fn scenario(id: &str, value: u8) -> MissionReference {
    MissionReference {
        revision_id: id.to_owned(),
        schema_version: flight_tune::MISSION_SCHEMA_VERSION,
        content_digest: fixed_digest(value),
        max_samples: 100,
        sample_timeout_ns: 20_000_000,
    }
}

fn artifact(id: &str, value: u8) -> ArtifactIdentity {
    ArtifactIdentity::new(id, fixed_digest(value)).expect("create artifact identity")
}

fn fixed_digest(value: u8) -> Digest {
    Digest::from_bytes([value; 32])
}

fn tuning_candidate(rate: f64) -> Candidate {
    Candidate::new(
        CandidateLineage {
            schema: "pilotage.test.candidate.v1".to_owned(),
            base_preset_digest: fixed_digest(2),
            plant_digest: fixed_digest(3),
        },
        BTreeMap::from([("rate".to_owned(), rate)]),
    )
    .expect("create fixture candidate")
}

fn candidate_digest(rate: f64) -> Digest {
    digest::document("candidate", &tuning_candidate(rate)).expect("fixture candidate digest")
}

pub(super) fn rewrite_hidden_attempt(
    evidence: &mut CampaignEvidence,
    role: AttemptRole,
    proof: &AuthenticatedEvaluationProof,
) {
    authority::rewrite_hidden_attempt(evidence, role, proof);
}

pub(super) fn rewrite_promotion_authority(
    evidence: &mut CampaignEvidence,
    frozen_candidate: Digest,
) {
    authority::rewrite_promotion_authority(evidence, frozen_candidate);
}

pub(super) fn rechain_journal_authority(evidence: &mut CampaignEvidence) {
    authority::rechain_journal_authority(evidence);
}

pub(super) fn assert_journal_chain_linked(evidence: &CampaignEvidence) {
    authority::assert_journal_chain_linked(evidence);
}
