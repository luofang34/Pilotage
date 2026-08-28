use std::collections::BTreeMap;

use flight_tune::{
    ArtifactIdentity, CandidateTransitionReference, RUN_TERMINAL_OPERATION_ORDER,
    RunBindingReceipt, RunExecutionContext, RunRecord, RunTerminalClass, RunTerminalDiagnostic,
    RunTerminalIntent, RunTerminalOperation, RunTerminalOperationOutcome, RunTerminalPlan,
    RunTerminalReceipt, RunTerminalRecoveryState, RunTerminalReport, RunTerminalScope,
    RunTerminalSemanticOutcome,
};

use crate::digest;

use super::super::super::plan;
use super::{Point, fixed_digest};

pub(super) fn run(
    expected: &plan::ExpectedRun<'_>,
    point: Point,
    missing_objective: bool,
) -> RunRecord {
    RunRecord {
        scenario_set: expected.scenario_set,
        mission_revision_id: expected.scenario.revision_id.clone(),
        repetition: expected.repetition,
        seed: expected.seed,
        loss: point.loss,
        control_effort: point.effort,
        objectives: if missing_objective {
            BTreeMap::new()
        } else {
            BTreeMap::from([("tracking".to_owned(), point.objective)])
        },
        passed_hard_gates: vec!["crash".to_owned(), "finite".to_owned()],
    }
}

pub(super) fn receipt(
    expected: &plan::ExpectedRun<'_>,
    run: RunRecord,
    vehicle: &ArtifactIdentity,
    transition: Option<CandidateTransitionReference>,
) -> RunTerminalReceipt {
    terminal_receipt(expected, run, vehicle, successful_outcomes(), transition)
}

pub(super) fn quarantine_receipt(
    expected: &plan::ExpectedRun<'_>,
    run: RunRecord,
    vehicle: &ArtifactIdentity,
) -> RunTerminalReceipt {
    let mut outcomes = successful_outcomes();
    let diagnostic =
        RunTerminalDiagnostic::new("control stop failed").expect("create terminal diagnostic");
    outcomes[1] =
        RunTerminalOperationOutcome::failed(RunTerminalOperation::ControlStop, diagnostic)
            .expect("create terminal failure");
    terminal_receipt(expected, run, vehicle, outcomes, None)
}

fn terminal_receipt(
    expected: &plan::ExpectedRun<'_>,
    run: RunRecord,
    vehicle: &ArtifactIdentity,
    outcomes: Vec<RunTerminalOperationOutcome>,
    transition: Option<CandidateTransitionReference>,
) -> RunTerminalReceipt {
    let context = RunExecutionContext::new(
        expected.session_digest,
        expected.trial_id,
        expected.role,
        expected.candidate,
        // A challenger run is the one role whose identity is incomplete
        // without the authorization that moved the vehicle onto it, and the
        // context refuses to be built without one.
        transition,
        expected.scenario_set,
        expected.scenario,
        expected.repetition,
        expected.seed,
    )
    .expect("create run context");
    let intent = RunTerminalIntent::new(
        &context,
        digest::domain(
            "run execution context",
            b"flight-tune:run-execution-context:v2\0",
            &context,
        )
        .expect("run intent digest"),
        RunTerminalSemanticOutcome::ScenarioComplete {
            candidate_digest: expected.candidate,
            mission_content_digest: expected.scenario.content_digest,
            run,
        },
    )
    .expect("create terminal intent");
    let terminal_plan =
        RunTerminalPlan::new(RunTerminalScope::Active).expect("create terminal plan");
    let report = RunTerminalReport::new(
        &terminal_plan,
        &intent,
        RunTerminalRecoveryState::Live,
        outcomes,
    )
    .expect("create terminal report");
    let class = RunTerminalClass::classify(&intent, &report).expect("classify terminal report");
    let binding = RunBindingReceipt::new(&context, &terminal_plan, vehicle.clone())
        .expect("create run binding");
    RunTerminalReceipt::new(&binding, &intent, &report, class, fixed_digest(91))
        .expect("create terminal receipt")
}

fn successful_outcomes() -> Vec<RunTerminalOperationOutcome> {
    RUN_TERMINAL_OPERATION_ORDER
        .into_iter()
        .map(|operation| {
            let durable =
                (operation == RunTerminalOperation::ChildTerminate).then_some(fixed_digest(92));
            RunTerminalOperationOutcome::succeeded(operation, durable)
                .expect("create terminal success")
        })
        .collect()
}
