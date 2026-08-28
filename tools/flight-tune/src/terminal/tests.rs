#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use crate::{
    ArtifactIdentity, AttemptRole, Digest, GateOutcome, HardGateFailure, MissionReference,
    RunExecutionContext, RunRecord, ScenarioSet,
};

use super::{
    RUN_TERMINAL_OPERATION_ORDER, RunBindingReceipt, RunTerminalClass, RunTerminalIntent,
    RunTerminalOperationOutcome, RunTerminalPlan, RunTerminalReceipt, RunTerminalRecoveryState,
    RunTerminalReport, RunTerminalScope, RunTerminalSemanticOutcome,
};

mod receipt;
mod report;
mod tamper;

#[derive(Clone, Copy)]
pub(super) enum SemanticCase {
    ScenarioComplete,
    HardGateAbort,
}

pub(super) fn fixed_digest(value: u8) -> Digest {
    Digest::from_bytes([value; 32])
}

pub(super) fn run_context() -> RunExecutionContext {
    let scenario = MissionReference {
        revision_id: "step-calm".to_owned(),
        schema_version: flight_tune::MISSION_SCHEMA_VERSION,
        content_digest: fixed_digest(2),
        max_samples: 100,
        sample_timeout_ns: 20_000_000,
    };
    RunExecutionContext::new(
        fixed_digest(1),
        7,
        AttemptRole::TrainingBaseline,
        fixed_digest(3),
        None,
        ScenarioSet::Training,
        &scenario,
        0,
        41,
    )
    .expect("create run context")
}

pub(super) fn terminal_intent(case: SemanticCase) -> RunTerminalIntent {
    let context = run_context();
    let outcome = match case {
        SemanticCase::ScenarioComplete => RunTerminalSemanticOutcome::ScenarioComplete {
            candidate_digest: context.candidate_digest(),
            mission_content_digest: context.mission_content_digest(),
            run: run_record(),
        },
        SemanticCase::HardGateAbort => RunTerminalSemanticOutcome::HardGateAbort {
            candidate_digest: context.candidate_digest(),
            mission_content_digest: context.mission_content_digest(),
            failure: hard_gate_failure(),
        },
    };
    RunTerminalIntent::new(
        &context,
        context.digest().expect("digest run context"),
        outcome,
    )
    .expect("create terminal intent")
}

pub(super) fn active_plan() -> RunTerminalPlan {
    RunTerminalPlan::new(RunTerminalScope::Active).expect("create terminal plan")
}

pub(super) fn successful_outcomes() -> Vec<RunTerminalOperationOutcome> {
    RUN_TERMINAL_OPERATION_ORDER
        .into_iter()
        .map(|operation| {
            let proof = (operation == super::RunTerminalOperation::ChildTerminate)
                .then_some(fixed_digest(90));
            RunTerminalOperationOutcome::succeeded(operation, proof)
                .expect("create operation success")
        })
        .collect()
}

pub(super) fn terminal_report(
    intent: &RunTerminalIntent,
    outcomes: Vec<RunTerminalOperationOutcome>,
) -> RunTerminalReport {
    RunTerminalReport::new(
        &active_plan(),
        intent,
        RunTerminalRecoveryState::Live,
        outcomes,
    )
    .expect("create terminal report")
}

pub(super) fn binding(intent: &RunTerminalIntent) -> RunBindingReceipt {
    let adapter = ArtifactIdentity::new("reference-terminal-adapter", fixed_digest(91))
        .expect("create adapter identity");
    RunBindingReceipt::new(intent.context(), &active_plan(), adapter).expect("create run binding")
}

pub(super) fn terminal_receipt(case: SemanticCase) -> RunTerminalReceipt {
    let intent = terminal_intent(case);
    let report = terminal_report(&intent, successful_outcomes());
    let class = RunTerminalClass::classify(&intent, &report).expect("classify terminal report");
    RunTerminalReceipt::new(&binding(&intent), &intent, &report, class, fixed_digest(92))
        .expect("create terminal receipt")
}

pub(super) fn run_record() -> RunRecord {
    RunRecord {
        scenario_set: ScenarioSet::Training,
        mission_revision_id: "step-calm".to_owned(),
        repetition: 0,
        seed: 41,
        loss: 0.2,
        control_effort: 0.3,
        objectives: BTreeMap::from([("tracking".to_owned(), 0.2)]),
        passed_hard_gates: vec!["crash".to_owned()],
    }
}

pub(super) fn hard_gate_failure() -> HardGateFailure {
    HardGateFailure {
        scenario_set: ScenarioSet::Training,
        mission_revision_id: "step-calm".to_owned(),
        repetition: 0,
        seed: 41,
        sample_sequence: 5,
        elapsed_ms: 80,
        gate: GateOutcome::fail("crash", "attitude limit"),
    }
}
