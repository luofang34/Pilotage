use std::collections::BTreeMap;

use crate::journal::storage::document_digest;
use crate::model::derive_seed;
use crate::score::aggregate_runs;
use crate::{
    AttemptRole, Candidate, CandidateEvaluation, CandidateTransitionReceipt,
    CandidateTransitionRequest, Digest, GateOutcome, HardGateFailure, Journal, JournalEvent,
    OperationStatus, RunBindingReceipt, RunExecutionContext, RunRecord, RunTerminalClass,
    RunTerminalDiagnostic, RunTerminalIntent, RunTerminalOperation, RunTerminalOperationOutcome,
    RunTerminalPlan, RunTerminalReceipt, RunTerminalRecoveryState, RunTerminalReport,
    RunTerminalScope, RunTerminalSemanticOutcome, ScenarioSet, SearchStage, TuneError,
};

pub(super) struct EvidenceFailureFixture {
    pub(super) trial_id: u64,
    pub(super) receipt: RunTerminalReceipt,
}

pub(super) fn record_passing_baseline(
    journal: &mut Journal,
    stage: &SearchStage,
    candidate: &Candidate,
) {
    let (trial_id, candidate_digest) = prepare_baseline(journal, stage, candidate);
    let runs = (0..stage.repetitions)
        .map(|repetition| {
            let context =
                prepare_training_run(journal, stage, trial_id, candidate_digest, repetition);
            let run = run_record(stage, &context, 0.5);
            commit_run(
                journal,
                u64::from(repetition),
                &context,
                RunTerminalSemanticOutcome::ScenarioComplete {
                    candidate_digest,
                    scenario_digest: context.scenario_digest(),
                    run: run.clone(),
                },
            );
            run
        })
        .collect::<Vec<_>>();
    let aggregate = aggregate_runs(&runs, ScenarioSet::Training).expect("aggregate baseline");
    journal
        .complete_attempt(
            trial_id,
            CandidateEvaluation::Passed { aggregate, runs },
            Some(true),
        )
        .expect("complete passing baseline");
    record_successful_cleanup(journal, trial_id);
}

pub(super) fn record_hard_gate_failed_baseline(
    journal: &mut Journal,
    stage: &SearchStage,
    candidate: &Candidate,
) {
    let (trial_id, candidate_digest) = prepare_baseline(journal, stage, candidate);
    let context = prepare_training_run(journal, stage, trial_id, candidate_digest, 0);
    let failure = HardGateFailure {
        scenario_set: ScenarioSet::Training,
        scenario_id: context.scenario_id().to_owned(),
        repetition: 0,
        seed: context.seed(),
        sample_sequence: 1,
        elapsed_ms: 10,
        gate: GateOutcome::fail("envelope", "the vehicle left the test envelope"),
    };
    commit_run(
        journal,
        0,
        &context,
        RunTerminalSemanticOutcome::HardGateAbort {
            candidate_digest,
            scenario_digest: context.scenario_digest(),
            failure: failure.clone(),
        },
    );
    journal
        .complete_attempt(
            trial_id,
            CandidateEvaluation::HardGateFailed {
                failure,
                completed_runs: Vec::new(),
            },
            Some(false),
        )
        .expect("complete failed baseline");
    record_successful_cleanup(journal, trial_id);
}

pub(super) fn record_quarantined_baseline(
    journal: &mut Journal,
    stage: &SearchStage,
    candidate: &Candidate,
) {
    let (trial_id, candidate_digest) = prepare_baseline(journal, stage, candidate);
    let context = prepare_training_run(journal, stage, trial_id, candidate_digest, 0);
    let diagnostic = RunTerminalDiagnostic::new("baseline simulator failure")
        .expect("create execution diagnostic");
    commit_run(
        journal,
        0,
        &context,
        RunTerminalSemanticOutcome::ExecutionError { diagnostic },
    );
    journal
        .quarantine_attempt(trial_id)
        .expect("quarantine baseline");
    record_successful_cleanup(journal, trial_id);
}

pub(super) fn record_evidence_failure_without_commit(
    journal: &mut Journal,
    stage: &SearchStage,
    candidate: &Candidate,
) -> EvidenceFailureFixture {
    let (trial_id, candidate_digest) = prepare_baseline(journal, stage, candidate);
    let context = prepare_training_run(journal, stage, trial_id, candidate_digest, 0);
    let run = run_record(stage, &context, 0.5);
    let outcome = RunTerminalSemanticOutcome::ScenarioComplete {
        candidate_digest,
        scenario_digest: context.scenario_digest(),
        run,
    };
    let plan = RunTerminalPlan::new(RunTerminalScope::Active).expect("create terminal plan");
    let binding =
        RunBindingReceipt::new(&context, &plan, journal.session().runtimes.vehicle.clone())
            .expect("create run binding");
    let intent = RunTerminalIntent::new(
        &context,
        context.digest().expect("digest run context"),
        outcome,
    )
    .expect("create terminal intent");
    let report = RunTerminalReport::new(
        &plan,
        &intent,
        RunTerminalRecoveryState::Live,
        successful_outcomes(),
    )
    .expect("create terminal report");
    let base_class = RunTerminalClass::classify(&intent, &report).expect("classify report");
    let class =
        RunTerminalClass::evidence_failure(&intent, &report).expect("classify evidence failure");
    let receipt = RunTerminalReceipt::new(
        &binding,
        &intent,
        &report,
        class,
        Digest::from_bytes([93; 32]),
    )
    .expect("create evidence failure receipt");
    journal
        .bind_run_terminal(trial_id, 0, plan, binding)
        .expect("bind terminal run");
    journal
        .prepare_run_terminal_intent(trial_id, 0, intent)
        .expect("save terminal intent");
    journal
        .record_run_terminal_report(trial_id, 0, report, base_class)
        .expect("save terminal report");
    journal
        .record_run_terminal_evidence_failure(trial_id, 0, class)
        .expect("save evidence failure");
    EvidenceFailureFixture { trial_id, receipt }
}

pub(super) fn reject_challenger_authorization(
    journal: &mut Journal,
    stage: &SearchStage,
    source: &Candidate,
    target: &Candidate,
) {
    let receipt = transition_receipt(journal, stage, source, target, 0);
    let entry_count = journal.entries().len();
    let error = journal
        .authorize_training_transition(0, "increase gain", target, receipt)
        .expect_err("reject challenger authorization");

    assert!(matches!(error, TuneError::InvalidJournal { .. }));
    assert_eq!(journal.entries().len(), entry_count);
    assert_no_transition_authorization(journal);
}

pub(super) fn assert_no_transition_authorization(journal: &Journal) {
    assert!(journal.entries().iter().all(|entry| !matches!(
        entry.event,
        JournalEvent::CandidateTransitionAuthorized { .. }
    )));
}

fn prepare_baseline(
    journal: &mut Journal,
    stage: &SearchStage,
    candidate: &Candidate,
) -> (u64, Digest) {
    let candidate_digest = document_digest("candidate", candidate).expect("candidate digest");
    let role = AttemptRole::TrainingBaseline;
    let plan = role
        .plan_digest(stage, candidate_digest, journal.session().fixed_seed)
        .expect("baseline plan");
    journal
        .prepare_attempt(role, candidate, plan, None)
        .expect("prepare baseline")
}

fn prepare_training_run(
    journal: &mut Journal,
    stage: &SearchStage,
    trial_id: u64,
    candidate_digest: Digest,
    repetition: u32,
) -> RunExecutionContext {
    let scenario = &stage.training_scenarios[0];
    let seed = derive_seed(
        journal.session().fixed_seed,
        ScenarioSet::Training,
        scenario,
        repetition,
    );
    let context = RunExecutionContext::new(
        journal.session_digest().expect("session digest"),
        trial_id,
        AttemptRole::TrainingBaseline,
        candidate_digest,
        None,
        ScenarioSet::Training,
        scenario,
        repetition,
        seed,
    )
    .expect("baseline run context");
    journal
        .prepare_run(u64::from(repetition), &context)
        .expect("prepare baseline run");
    context
}

fn commit_run(
    journal: &mut Journal,
    run_index: u64,
    context: &RunExecutionContext,
    outcome: RunTerminalSemanticOutcome,
) {
    let plan = RunTerminalPlan::new(RunTerminalScope::Active).expect("create terminal plan");
    let binding =
        RunBindingReceipt::new(context, &plan, journal.session().runtimes.vehicle.clone())
            .expect("create run binding");
    let intent = RunTerminalIntent::new(
        context,
        context.digest().expect("digest run context"),
        outcome,
    )
    .expect("create terminal intent");
    let report = RunTerminalReport::new(
        &plan,
        &intent,
        RunTerminalRecoveryState::Live,
        successful_outcomes(),
    )
    .expect("create terminal report");
    let class = RunTerminalClass::classify(&intent, &report).expect("classify terminal report");
    let receipt = RunTerminalReceipt::new(
        &binding,
        &intent,
        &report,
        class,
        Digest::from_bytes([92; 32]),
    )
    .expect("create terminal receipt");
    journal
        .bind_run_terminal(context.trial_id(), run_index, plan, binding)
        .expect("bind terminal run");
    journal
        .prepare_run_terminal_intent(context.trial_id(), run_index, intent)
        .expect("save terminal intent");
    journal
        .record_run_terminal_report(context.trial_id(), run_index, report, class)
        .expect("save terminal report");
    journal
        .commit_run(context.trial_id(), run_index, receipt)
        .expect("commit terminal run");
}

fn successful_outcomes() -> Vec<RunTerminalOperationOutcome> {
    crate::RUN_TERMINAL_OPERATION_ORDER
        .into_iter()
        .map(|operation| {
            let proof = (operation == RunTerminalOperation::ChildTerminate)
                .then_some(Digest::from_bytes([90; 32]));
            RunTerminalOperationOutcome::succeeded(operation, proof)
                .expect("create terminal success")
        })
        .collect()
}

fn run_record(stage: &SearchStage, context: &RunExecutionContext, loss: f64) -> RunRecord {
    RunRecord {
        scenario_set: context.scenario_set(),
        scenario_id: context.scenario_id().to_owned(),
        repetition: context.repetition(),
        seed: context.seed(),
        loss,
        control_effort: 0.2,
        objectives: BTreeMap::new(),
        passed_hard_gates: stage.required_hard_gates.clone(),
    }
}

fn transition_receipt(
    journal: &Journal,
    stage: &SearchStage,
    source: &Candidate,
    target: &Candidate,
    attempt_index: u64,
) -> CandidateTransitionReceipt {
    let source_digest = document_digest("candidate", source).expect("source digest");
    let target_digest = document_digest("candidate", target).expect("target digest");
    let plan = AttemptRole::TrainingChallenger { attempt_index }
        .plan_digest(stage, target_digest, journal.session().fixed_seed)
        .expect("challenger plan");
    let planning = crate::adapter::planning_context_digest(journal.session().stage_digest, plan)
        .expect("planning context");
    let request = CandidateTransitionRequest::new(
        journal.session_digest().expect("session digest"),
        source,
        source_digest,
        target,
        target_digest,
        journal.session().runtimes.transition_validator.clone(),
        journal.session().runtimes.adjacency_policy_digest,
        planning,
    )
    .expect("transition request");
    CandidateTransitionReceipt::authorized(&request).expect("transition receipt")
}

fn record_successful_cleanup(journal: &mut Journal, trial_id: u64) {
    journal
        .record_cleanup(trial_id, OperationStatus::Succeeded)
        .expect("record successful cleanup");
}
