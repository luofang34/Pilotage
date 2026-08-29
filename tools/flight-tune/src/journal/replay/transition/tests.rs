#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use super::{authorize, validate_attempt};
use crate::adapter::planning_context_digest;
use crate::identity::harness_build_identity;
use crate::journal::replay::run;
use crate::journal::replay::{PendingAttempt, initial_state};
use crate::journal::storage::document_digest;
use crate::model::derive_seed;
use crate::{
    ArtifactIdentity, AttemptRole, Candidate, CandidateEvaluation, CandidateLineage,
    CandidateTransitionReceipt, CandidateTransitionRequest, ConfidenceInterval, Digest,
    MissionReference, ParameterBounds, PromotionPolicy, QualificationPolicy, RunExecutionContext,
    RuntimeIdentities, ScoreAggregate, SearchStage, SessionIdentity, TuneError,
};

#[test]
fn replay_keeps_the_exact_authorized_target_and_reference() {
    let fixture = fixture();
    let receipt = receipt(&fixture, fixture.policy, fixture.planning);
    let reference = receipt.reference();
    let mut state = searching_state(fixture.source_digest);

    authorize(
        &mut state,
        0,
        "increase gain",
        fixture.target_digest,
        &receipt,
        &fixture.stage,
        &fixture.session,
    )
    .expect("authorize exact transition");

    let authorized = state
        .authorized_transition
        .as_ref()
        .expect("keep authorization for crash recovery");
    assert_eq!(authorized.attempt_index, 0);
    assert_eq!(authorized.candidate, fixture.target_digest);
    assert_eq!(authorized.reference, reference);
    validate_attempt(
        &state,
        AttemptRole::TrainingChallenger { attempt_index: 0 },
        fixture.target_digest,
        Some(&reference),
    )
    .expect("bind exact authorization to attempt");
}

#[test]
fn replay_rejects_a_self_consistent_forged_policy_and_context() {
    let fixture = fixture();
    let forged = receipt(&fixture, digest(91), digest(92));
    let mut state = searching_state(fixture.source_digest);

    let error = authorize(
        &mut state,
        0,
        "increase gain",
        fixture.target_digest,
        &forged,
        &fixture.stage,
        &fixture.session,
    )
    .expect_err("reject forged policy and context");

    assert!(matches!(error, TuneError::InvalidJournal { .. }));
    assert!(state.authorized_transition.is_none());
}

#[test]
fn replay_rejects_a_challenger_without_one_matching_authorization() {
    let fixture = fixture();
    let state = searching_state(fixture.source_digest);

    let error = validate_attempt(
        &state,
        AttemptRole::TrainingChallenger { attempt_index: 0 },
        fixture.target_digest,
        None,
    )
    .expect_err("reject missing authorization");

    assert!(matches!(error, TuneError::InvalidJournal { .. }));
}

#[test]
fn challenger_role_requires_a_passed_baseline_and_incumbent() {
    let fixture = fixture();
    let role = AttemptRole::TrainingChallenger { attempt_index: 0 };
    let mut state = searching_state(fixture.source_digest);
    assert!(super::super::attempt::role_allowed(
        &state,
        role,
        fixture.target_digest,
        fixture.source_digest,
    ));

    state.training_incumbent_evaluation = None;
    assert!(!super::super::attempt::role_allowed(
        &state,
        role,
        fixture.target_digest,
        fixture.source_digest,
    ));

    state.training_incumbent_evaluation = Some(passed_evaluation());
    state.training_baseline = Some(CandidateEvaluation::Quarantined {
        reason: "unsafe baseline".to_owned(),
    });
    assert!(!super::super::attempt::role_allowed(
        &state,
        role,
        fixture.target_digest,
        fixture.source_digest,
    ));
}

#[test]
fn replay_rejects_a_coherently_forged_run_transition_reference() {
    let fixture = fixture();
    let exact_receipt = receipt(&fixture, fixture.policy, fixture.planning);
    let exact_reference = exact_receipt.reference();
    let forged_reference = receipt(&fixture, digest(91), digest(92)).reference();
    let role = AttemptRole::TrainingChallenger { attempt_index: 0 };
    let mut state = searching_state(fixture.source_digest);
    state.pending = Some(PendingAttempt {
        retry_index: 0,
        retry_decision: None,
        trial_id: 0,
        role,
        candidate: fixture.target_digest,
        plan_digest: role
            .plan_digest(
                &fixture.stage,
                fixture.target_digest,
                fixture.session.fixed_seed,
            )
            .expect("run plan digest"),
        transition: Some(exact_reference),
        prepared_runs: Vec::new(),
        outcome: None,
    });
    let scenario = &fixture.stage.training_scenarios[0];
    let context = RunExecutionContext::new(
        document_digest("session identity", &fixture.session).expect("session digest"),
        0,
        role,
        fixture.target_digest,
        Some(forged_reference),
        crate::ScenarioSet::Training,
        scenario,
        0,
        derive_seed(
            fixture.session.fixed_seed,
            crate::ScenarioSet::Training,
            scenario,
            0,
        ),
        0,
    )
    .expect("self-consistent forged run context");
    let forged_digest = context.digest().expect("forged run digest");

    let error = run::prepare(
        &mut state,
        0,
        0,
        &context,
        forged_digest,
        &fixture.stage,
        &fixture.session,
    )
    .expect_err("reject changed transition reference");

    assert!(matches!(error, TuneError::InvalidJournal { .. }));
    assert!(
        state
            .pending
            .as_ref()
            .is_some_and(|pending| pending.prepared_runs.is_empty())
    );
}

struct Fixture {
    source: Candidate,
    source_digest: Digest,
    target: Candidate,
    target_digest: Digest,
    stage: SearchStage,
    session: SessionIdentity,
    policy: Digest,
    planning: Digest,
}

fn fixture() -> Fixture {
    let lineage = CandidateLineage {
        schema: "test-candidate-v1".to_owned(),
        base_preset_digest: digest(1),
        plant_digest: digest(2),
    };
    let source = candidate(&lineage, 0.4);
    let target = candidate(&lineage, 0.5);
    let source_digest = document_digest("candidate", &source).expect("source digest");
    let target_digest = document_digest("candidate", &target).expect("target digest");
    let stage = stage();
    let stage_digest = document_digest("search stage", &stage).expect("stage digest");
    let policy = digest(19);
    let session = SessionIdentity {
        stage_digest,
        initial_candidate_digest: source_digest,
        candidate_lineage: lineage,
        fixed_seed: 41,
        runtimes: runtimes(policy),
    };
    let plan = AttemptRole::TrainingChallenger { attempt_index: 0 }
        .plan_digest(&stage, target_digest, session.fixed_seed)
        .expect("plan digest");
    let planning = planning_context_digest(stage_digest, plan).expect("planning context");
    Fixture {
        source,
        source_digest,
        target,
        target_digest,
        stage,
        session,
        policy,
        planning,
    }
}

fn receipt(fixture: &Fixture, policy: Digest, planning: Digest) -> CandidateTransitionReceipt {
    let session_digest = document_digest("session identity", &fixture.session).expect("session");
    let request = CandidateTransitionRequest::new(
        session_digest,
        &fixture.source,
        fixture.source_digest,
        &fixture.target,
        fixture.target_digest,
        fixture.session.runtimes.transition_validator.clone(),
        policy,
        planning,
    )
    .expect("transition request");
    CandidateTransitionReceipt::authorized(&request).expect("transition receipt")
}

fn searching_state(source: Digest) -> crate::journal::replay::JournalState {
    let mut state = initial_state(source);
    let evaluation = passed_evaluation();
    state.training_baseline = Some(evaluation.clone());
    state.training_incumbent_evaluation = Some(evaluation);
    state
}

fn passed_evaluation() -> CandidateEvaluation {
    CandidateEvaluation::Passed {
        aggregate: ScoreAggregate {
            run_count: 2,
            mean_loss: 0.5,
            p95_loss: 0.5,
            loss_variance: 0.0,
            loss_confidence_95: ConfidenceInterval {
                lower: 0.5,
                upper: 0.5,
            },
            mean_control_effort: 0.2,
        },
        runs: Vec::new(),
    }
}

fn candidate(lineage: &CandidateLineage, gain: f64) -> Candidate {
    Candidate::new(lineage.clone(), BTreeMap::from([("gain".to_owned(), gain)])).expect("candidate")
}

fn stage() -> SearchStage {
    let scenario = MissionReference {
        revision_id: "training".to_owned(),
        schema_version: flight_tune::MISSION_SCHEMA_VERSION,
        content_digest: digest(3),
        max_samples: 8,
        sample_timeout_ns: 100_000_000,
    };
    SearchStage {
        execution_retry: crate::ExecutionRetryPolicy::none(),
        id: "transition-stage".to_owned(),
        allowlist: BTreeMap::from([(
            "gain".to_owned(),
            ParameterBounds {
                minimum: 0.0,
                maximum: 1.0,
            },
        )]),
        fixed_parameters: BTreeMap::new(),
        required_hard_gates: vec!["envelope".to_owned()],
        training_scenarios: vec![scenario.clone()],
        promotion_scenarios: vec![scenario.clone()],
        final_qualification_scenarios: vec![scenario],
        repetitions: 1,
        promotion: PromotionPolicy {
            schema_version: crate::PROMOTION_POLICY_SCHEMA_VERSION,
            seed_policy: crate::PromotionSeedPolicy::PairedScenarioDigestV1,
            minimum_loss_improvement: 0.0,
            minimum_relative_loss_improvement: 0.0,
            maximum_control_effort_increase: 0.0,
            objective_regression_upper_95: BTreeMap::from([("tracking".to_owned(), 0.0)]),
        },
        qualification: QualificationPolicy {
            maximum_loss_confidence_upper: 1.0,
            maximum_p95_loss: 1.0,
            maximum_mean_control_effort: 1.0,
            objective_maxima: BTreeMap::new(),
        },
    }
}

fn runtimes(policy: Digest) -> RuntimeIdentities {
    RuntimeIdentities {
        harness_build: harness_build_identity(),
        strategy: identity("strategy", 11),
        metric: identity("metric", 12),
        hard_gates: identity("hard-gates", 13),
        scenario_runtime: Some(identity("pilotage-scenario-runtime-v2", 19)),
        simulator: identity("simulator", 14),
        airframe: identity("airframe", 15),
        vehicle: identity("vehicle", 16),
        transition_validator: identity("transition-validator", 17),
        adjacency_policy_digest: policy,
    }
}

fn identity(id: &str, byte: u8) -> ArtifactIdentity {
    ArtifactIdentity::new(id, digest(byte)).expect("artifact identity")
}

fn digest(byte: u8) -> Digest {
    Digest::from_bytes([byte; 32])
}
