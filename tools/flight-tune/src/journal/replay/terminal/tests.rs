#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use crate::identity::harness_build_identity;
use crate::journal::replay::{attempt, initial_state, run};
use crate::journal::snapshot::{PendingAttemptSnapshot, RunTerminalSnapshot};
use crate::journal::{AttemptRole, JournalEvent, SessionIdentity};
use crate::model::derive_seed;
use crate::{
    ArtifactIdentity, CandidateLineage, Digest, GateOutcome, HardGateFailure, MissionReference,
    ParameterBounds, PromotionPolicy, QualificationPolicy, RunBindingReceipt, RunExecutionContext,
    RunRecord, RunTerminalClass, RunTerminalDiagnostic, RunTerminalIntent, RunTerminalOperation,
    RunTerminalOperationOutcome, RunTerminalPlan, RunTerminalReceipt, RunTerminalRecoveryState,
    RunTerminalReport, RunTerminalScope, RunTerminalSemanticOutcome, RuntimeIdentities,
    ScenarioSet, SearchStage,
};

use super::{apply_event, quarantine_reason};

mod attempt_tests;
mod tamper_tests;

#[derive(Clone, Copy)]
pub(super) enum SemanticCase {
    ScenarioComplete,
    HardGateAbort,
    ExecutionError,
}

pub(super) struct ReplayFixture {
    pub(super) state: super::super::JournalState,
    pub(super) session: SessionIdentity,
    pub(super) stage: SearchStage,
}

pub(super) struct RunArtifacts {
    pub(super) index: u64,
    pub(super) plan: RunTerminalPlan,
    pub(super) binding: RunBindingReceipt,
    pub(super) intent: RunTerminalIntent,
    pub(super) report: RunTerminalReport,
    pub(super) base_class: RunTerminalClass,
    pub(super) receipt: RunTerminalReceipt,
}

impl ReplayFixture {
    pub(super) fn new() -> Self {
        let stage = test_stage();
        let candidate = fixed_digest(3);
        let session = test_session(&stage, candidate);
        let mut state = initial_state(candidate);
        let role = AttemptRole::TrainingBaseline { suite_index: 0 };
        let plan_digest = role
            .plan_digest(&stage, candidate, session.fixed_seed)
            .expect("create attempt plan");
        attempt::prepare(
            &mut state,
            0,
            role,
            candidate,
            plan_digest,
            None,
            &stage,
            candidate,
            session.fixed_seed,
        )
        .expect("prepare attempt");
        Self {
            state,
            session,
            stage,
        }
    }

    pub(super) fn prepare_run(&mut self, index: u64, case: SemanticCase) -> RunArtifacts {
        let context = self.context(index);
        let run_intent_digest = context.digest().expect("digest run context");
        run::prepare(
            &mut self.state,
            0,
            index,
            &context,
            run_intent_digest,
            &self.stage,
            &self.session,
        )
        .expect("prepare run");
        artifacts(&self.session, context, index, case)
    }

    pub(super) fn context(&self, index: u64) -> RunExecutionContext {
        let scenario = &self.stage.training_scenarios[0];
        let repetition = u32::try_from(index).expect("small repetition");
        let seed = derive_seed(
            self.session.fixed_seed,
            ScenarioSet::Training,
            scenario,
            repetition,
        );
        RunExecutionContext::new(
            crate::journal::storage::document_digest("session identity", &self.session)
                .expect("digest session"),
            0,
            AttemptRole::TrainingBaseline { suite_index: 0 },
            self.session.initial_candidate_digest,
            None,
            ScenarioSet::Training,
            scenario,
            repetition,
            seed,
            0,
        )
        .expect("create run context")
    }

    pub(super) fn commit(&mut self, artifacts: &RunArtifacts) {
        for event in terminal_events(artifacts) {
            apply_event(&mut self.state, &event, &self.session).expect("apply terminal event");
        }
    }
}

#[test]
fn exact_terminal_chain_reaches_one_committed_state() {
    let mut fixture = ReplayFixture::new();
    let artifacts = fixture.prepare_run(0, SemanticCase::ScenarioComplete);

    fixture.commit(&artifacts);

    let pending = fixture.state.pending.as_ref().expect("pending attempt");
    let snapshot = PendingAttemptSnapshot::from(pending);
    assert_eq!(snapshot.committed_prefix().len(), 1);
    assert!(matches!(
        snapshot.current_run().map(|run| &run.terminal),
        Some(RunTerminalSnapshot::Committed { receipt })
            if receipt.as_ref() == &artifacts.receipt
    ));
}

#[test]
fn terminal_events_reject_skips_repeats_and_reverse_order() {
    let mut fixture = ReplayFixture::new();
    let artifacts = fixture.prepare_run(0, SemanticCase::ScenarioComplete);
    let intent_event = JournalEvent::RunTerminalIntentPrepared {
        trial_id: 0,
        run_index: 0,
        intent: artifacts.intent.clone(),
    };
    assert!(apply_event(&mut fixture.state, &intent_event, &fixture.session).is_err());

    let bind_event = terminal_events(&artifacts).remove(0);
    apply_event(&mut fixture.state, &bind_event, &fixture.session).expect("bind run");
    assert!(apply_event(&mut fixture.state, &bind_event, &fixture.session).is_err());
}

#[test]
fn run_binding_requires_the_exact_session_vehicle_identity() {
    let mut fixture = ReplayFixture::new();
    let artifacts = fixture.prepare_run(0, SemanticCase::ScenarioComplete);
    let foreign =
        ArtifactIdentity::new("foreign-vehicle", fixed_digest(77)).expect("create foreign adapter");
    let binding = RunBindingReceipt::new(artifacts.intent.context(), &artifacts.plan, foreign)
        .expect("create foreign binding");
    let event = JournalEvent::RunBound {
        trial_id: 0,
        run_index: 0,
        terminal_plan: artifacts.plan,
        binding,
    };

    assert!(apply_event(&mut fixture.state, &event, &fixture.session).is_err());
}

#[test]
fn report_requires_the_recomputed_base_class() {
    let mut fixture = ReplayFixture::new();
    let artifacts = fixture.prepare_run(0, SemanticCase::ScenarioComplete);
    let mut events = terminal_events(&artifacts);
    apply_event(&mut fixture.state, &events.remove(0), &fixture.session).expect("bind run");
    apply_event(&mut fixture.state, &events.remove(0), &fixture.session).expect("save intent");
    let evidence = RunTerminalClass::evidence_failure(&artifacts.intent, &artifacts.report)
        .expect("create different class");
    let event = JournalEvent::RunTerminalReportRecorded {
        trial_id: 0,
        run_index: 0,
        report: Box::new(artifacts.report),
        base_class: evidence,
        expected_receipt: Box::new(artifacts.receipt),
    };

    assert!(apply_event(&mut fixture.state, &event, &fixture.session).is_err());
}

#[test]
fn evidence_failure_is_durable_before_its_quarantine_commit() {
    let mut fixture = ReplayFixture::new();
    let artifacts = fixture.prepare_run(0, SemanticCase::ScenarioComplete);
    let mut events = terminal_events(&artifacts);
    for event in events.drain(..3) {
        apply_event(&mut fixture.state, &event, &fixture.session).expect("save base chain");
    }
    let class = RunTerminalClass::evidence_failure(&artifacts.intent, &artifacts.report)
        .expect("create evidence failure");
    let evidence_event = JournalEvent::RunTerminalEvidenceFailureRecorded {
        trial_id: 0,
        run_index: 0,
        class,
    };
    apply_event(&mut fixture.state, &evidence_event, &fixture.session)
        .expect("save evidence failure");
    let quarantine = RunTerminalReceipt::new(
        &artifacts.binding,
        &artifacts.intent,
        &artifacts.report,
        class,
        artifacts.receipt.causal_evidence_digest(),
    )
    .expect("create quarantine receipt");
    let commit = JournalEvent::RunCommitted {
        trial_id: 0,
        run_index: 0,
        receipt: Box::new(quarantine.clone()),
    };

    apply_event(&mut fixture.state, &commit, &fixture.session).expect("commit evidence failure");
    assert!(
        quarantine_reason(&quarantine)
            .expect("derive reason")
            .contains("evidence_failure")
    );
}

#[test]
fn evidence_failure_rejects_a_base_quarantine() {
    let mut fixture = ReplayFixture::new();
    let artifacts = fixture.prepare_run(0, SemanticCase::ExecutionError);
    let mut events = terminal_events(&artifacts);
    for event in events.drain(..3) {
        apply_event(&mut fixture.state, &event, &fixture.session).expect("save base chain");
    }
    let event = JournalEvent::RunTerminalEvidenceFailureRecorded {
        trial_id: 0,
        run_index: 0,
        class: artifacts.base_class,
    };

    assert!(apply_event(&mut fixture.state, &event, &fixture.session).is_err());
}

#[test]
fn hard_gate_and_quarantine_commits_block_the_next_run() {
    for case in [SemanticCase::HardGateAbort, SemanticCase::ExecutionError] {
        let mut fixture = ReplayFixture::new();
        let artifacts = fixture.prepare_run(0, case);
        fixture.commit(&artifacts);
        let context = fixture.context(1);

        assert!(
            run::prepare(
                &mut fixture.state,
                0,
                1,
                &context,
                context.digest().expect("digest context"),
                &fixture.stage,
                &fixture.session,
            )
            .is_err()
        );
    }
}

pub(super) fn terminal_events(artifacts: &RunArtifacts) -> Vec<JournalEvent> {
    vec![
        JournalEvent::RunBound {
            trial_id: 0,
            run_index: artifacts.index,
            terminal_plan: artifacts.plan.clone(),
            binding: artifacts.binding.clone(),
        },
        JournalEvent::RunTerminalIntentPrepared {
            trial_id: 0,
            run_index: artifacts.index,
            intent: artifacts.intent.clone(),
        },
        JournalEvent::RunTerminalReportRecorded {
            trial_id: 0,
            run_index: artifacts.index,
            report: Box::new(artifacts.report.clone()),
            base_class: artifacts.base_class,
            expected_receipt: Box::new(artifacts.receipt.clone()),
        },
        JournalEvent::RunCommitted {
            trial_id: 0,
            run_index: artifacts.index,
            receipt: Box::new(artifacts.receipt.clone()),
        },
    ]
}

fn artifacts(
    session: &SessionIdentity,
    context: RunExecutionContext,
    index: u64,
    case: SemanticCase,
) -> RunArtifacts {
    let plan = RunTerminalPlan::new(RunTerminalScope::Active).expect("create terminal plan");
    let binding = RunBindingReceipt::new(&context, &plan, session.runtimes.vehicle.clone())
        .expect("create run binding");
    let outcome = semantic_outcome(&context, index, case);
    let intent =
        RunTerminalIntent::new(&context, context.digest().expect("digest context"), outcome)
            .expect("create terminal intent");
    let report = RunTerminalReport::new(
        &plan,
        &intent,
        RunTerminalRecoveryState::Live,
        successful_outcomes(),
    )
    .expect("create terminal report");
    let base_class = RunTerminalClass::classify(&intent, &report).expect("classify report");
    let receipt = RunTerminalReceipt::new(&binding, &intent, &report, base_class, fixed_digest(92))
        .expect("create terminal receipt");
    RunArtifacts {
        index,
        plan,
        binding,
        intent,
        report,
        base_class,
        receipt,
    }
}

fn semantic_outcome(
    context: &RunExecutionContext,
    index: u64,
    case: SemanticCase,
) -> RunTerminalSemanticOutcome {
    match case {
        SemanticCase::ScenarioComplete => RunTerminalSemanticOutcome::ScenarioComplete {
            candidate_digest: context.candidate_digest(),
            mission_content_digest: context.mission_content_digest(),
            run: run_record(context, index),
        },
        SemanticCase::HardGateAbort => RunTerminalSemanticOutcome::HardGateAbort {
            candidate_digest: context.candidate_digest(),
            mission_content_digest: context.mission_content_digest(),
            failure: hard_gate_failure(context),
        },
        SemanticCase::ExecutionError => RunTerminalSemanticOutcome::ExecutionError {
            diagnostic: RunTerminalDiagnostic::new("the run connection ended")
                .expect("create diagnostic"),
        },
    }
}

pub(super) fn run_record(context: &RunExecutionContext, index: u64) -> RunRecord {
    RunRecord {
        scenario_set: context.scenario_set(),
        mission_revision_id: context.mission_revision_id().to_owned(),
        repetition: context.repetition(),
        seed: context.seed(),
        loss: 0.2 + index as f64 / 10.0,
        control_effort: 0.3,
        objectives: BTreeMap::from([("tracking".to_owned(), 0.2)]),
        passed_hard_gates: vec!["crash".to_owned()],
    }
}

pub(super) fn hard_gate_failure(context: &RunExecutionContext) -> HardGateFailure {
    HardGateFailure {
        scenario_set: context.scenario_set(),
        mission_revision_id: context.mission_revision_id().to_owned(),
        repetition: context.repetition(),
        seed: context.seed(),
        sample_sequence: 5,
        elapsed_ms: 80,
        gate: GateOutcome::fail("crash", "attitude limit"),
    }
}

fn successful_outcomes() -> Vec<RunTerminalOperationOutcome> {
    crate::RUN_TERMINAL_OPERATION_ORDER
        .into_iter()
        .map(|operation| {
            let proof =
                (operation == RunTerminalOperation::ChildTerminate).then_some(fixed_digest(90));
            RunTerminalOperationOutcome::succeeded(operation, proof)
                .expect("create operation success")
        })
        .collect()
}

fn test_session(stage: &SearchStage, candidate: Digest) -> SessionIdentity {
    SessionIdentity {
        stage_digest: crate::journal::storage::document_digest("search stage", stage)
            .expect("digest stage"),
        initial_candidate_digest: candidate,
        candidate_lineage: CandidateLineage {
            schema: "test-candidate-v1".to_owned(),
            base_preset_digest: fixed_digest(4),
            plant_digest: fixed_digest(5),
        },
        fixed_seed: 41,
        runtimes: test_runtimes(),
    }
}

fn test_stage() -> SearchStage {
    SearchStage {
        execution_retry: crate::ExecutionRetryPolicy::none(),
        id: "terminal-replay".to_owned(),
        allowlist: BTreeMap::from([(
            "gain".to_owned(),
            ParameterBounds {
                minimum: 0.0,
                maximum: 1.0,
            },
        )]),
        fixed_parameters: BTreeMap::new(),
        required_hard_gates: vec!["crash".to_owned()],
        training_scenarios: vec![scenario("training", 1)],
        training_suites: vec![crate::TrainingSuite {
            schema_version: crate::TRAINING_SUITE_SCHEMA_VERSION,
            id: "terminal-suite".to_owned(),
            primary_scenarios: vec![scenario("training", 1)],
            guard_scenarios: Vec::new(),
            guard_regression_limits: BTreeMap::new(),
            repetitions: 2,
        }],
        search_groups: vec![crate::SearchGroup {
            id: "terminal-group".to_owned(),
            kind: crate::SearchGroupKind::Controller,
            parameters: std::collections::BTreeSet::from(["gain".to_owned()]),
            suite_id: "terminal-suite".to_owned(),
        }],
        promotion_scenarios: vec![scenario("promotion", 2)],
        final_qualification_scenarios: vec![scenario("qualification", 3)],
        repetitions: 2,
        promotion: PromotionPolicy {
            schema_version: crate::PROMOTION_POLICY_SCHEMA_VERSION,
            seed_policy: crate::PromotionSeedPolicy::PairedScenarioDigestV1,
            minimum_loss_improvement: 0.0,
            minimum_relative_loss_improvement: 0.1,
            maximum_control_effort_increase: 0.2,
            objective_regression_upper_95: BTreeMap::from([("tracking".to_owned(), 0.2)]),
        },
        qualification: QualificationPolicy {
            maximum_loss_confidence_upper: 1.0,
            maximum_p95_loss: 1.0,
            maximum_mean_control_effort: 1.0,
            objective_maxima: BTreeMap::from([("tracking".to_owned(), 1.0)]),
        },
    }
}

fn scenario(id: &str, byte: u8) -> MissionReference {
    MissionReference {
        revision_id: id.to_owned(),
        schema_version: flight_tune::MISSION_SCHEMA_VERSION,
        content_digest: fixed_digest(byte),
        max_samples: 100,
        sample_timeout_ns: 20_000_000,
    }
}

fn test_runtimes() -> RuntimeIdentities {
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
        adjacency_policy_digest: fixed_digest(18),
    }
}

fn identity(id: &str, byte: u8) -> ArtifactIdentity {
    ArtifactIdentity::new(id, fixed_digest(byte)).expect("create identity")
}

fn fixed_digest(byte: u8) -> Digest {
    Digest::from_bytes([byte; 32])
}
