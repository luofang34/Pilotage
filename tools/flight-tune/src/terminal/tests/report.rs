use serde_json::Value;

use super::{
    SemanticCase, active_plan, fixed_digest, hard_gate_failure, run_context, run_record,
    successful_outcomes, terminal_intent, terminal_report,
};
use crate::terminal::{
    RUN_TERMINAL_OPERATION_ORDER, RunTerminalBindingStatus, RunTerminalClass,
    RunTerminalDiagnostic, RunTerminalDisposition, RunTerminalIntent, RunTerminalOperation,
    RunTerminalOperationOutcome, RunTerminalOperationStatus, RunTerminalPlan,
    RunTerminalQuarantine, RunTerminalRecoveryState, RunTerminalReport, RunTerminalScope,
    RunTerminalSemanticOutcome,
};

#[test]
fn both_successful_semantics_are_completed() {
    for case in [SemanticCase::ScenarioComplete, SemanticCase::HardGateAbort] {
        let intent = terminal_intent(case);
        let report = terminal_report(&intent, successful_outcomes());
        let class = RunTerminalClass::classify(&intent, &report).expect("classify report");
        assert!(class.is_completed());
    }
}

#[test]
fn binding_failure_quarantines_without_changing_operation_results() {
    for case in [SemanticCase::ScenarioComplete, SemanticCase::HardGateAbort] {
        let plan = active_plan();
        let intent = terminal_intent(case);
        let diagnostic = RunTerminalDiagnostic::new("terminal binding failed")
            .expect("create binding diagnostic");
        let report = RunTerminalReport::new_with_binding_status(
            &plan,
            &intent,
            RunTerminalRecoveryState::Live,
            RunTerminalBindingStatus::Failed { diagnostic },
            successful_outcomes(),
        )
        .expect("create binding failure report");
        let class = RunTerminalClass::classify(&intent, &report).expect("classify report");
        assert_terminal_failure(class);
        assert!(report.operations().iter().all(|outcome| matches!(
            outcome.status(),
            RunTerminalOperationStatus::Succeeded { .. }
        )));
    }
}

#[test]
fn changed_valid_binding_diagnostic_fails_the_report_digest() {
    let plan = active_plan();
    let intent = terminal_intent(SemanticCase::ScenarioComplete);
    let diagnostic =
        RunTerminalDiagnostic::new("terminal binding failed").expect("create binding diagnostic");
    let report = RunTerminalReport::new_with_binding_status(
        &plan,
        &intent,
        RunTerminalRecoveryState::Live,
        RunTerminalBindingStatus::Failed { diagnostic },
        successful_outcomes(),
    )
    .expect("create binding failure report");
    let mut document = serde_json::to_value(report).expect("encode report");
    document["binding_status"]["diagnostic"] = serde_json::to_value(
        RunTerminalDiagnostic::new("a different binding failure").expect("changed diagnostic"),
    )
    .expect("encode changed diagnostic");

    let changed: RunTerminalReport =
        serde_json::from_value(document).expect("decode changed report");
    assert!(changed.validate().is_err());
}

#[test]
fn report_schema_missing_binding_and_unknown_fields_fail_closed() {
    let intent = terminal_intent(SemanticCase::ScenarioComplete);
    let report = terminal_report(&intent, successful_outcomes());
    let document = serde_json::to_value(report).expect("encode report");

    let mut old_schema = document.clone();
    old_schema["schema_version"] = Value::from(1);
    let old_report: RunTerminalReport =
        serde_json::from_value(old_schema).expect("decode old report schema");
    assert!(old_report.validate().is_err());

    let mut missing_binding = document.clone();
    missing_binding
        .as_object_mut()
        .expect("report object")
        .remove("binding_status");
    assert!(serde_json::from_value::<RunTerminalReport>(missing_binding).is_err());

    let mut unknown_field = document;
    unknown_field
        .as_object_mut()
        .expect("report object")
        .insert("unreviewed_field".to_owned(), Value::Bool(true));
    assert!(serde_json::from_value::<RunTerminalReport>(unknown_field).is_err());
}

#[test]
fn execution_error_is_quarantine_after_successful_containment() {
    let context = run_context();
    let diagnostic = RunTerminalDiagnostic::new("control connection ended")
        .expect("create execution diagnostic");
    let intent = super::super::RunTerminalIntent::new(
        &context,
        context.digest().expect("digest context"),
        RunTerminalSemanticOutcome::ExecutionError { diagnostic },
    )
    .expect("create execution intent");
    let report = terminal_report(&intent, successful_outcomes());
    let class = RunTerminalClass::classify(&intent, &report).expect("classify report");
    assert_eq!(
        class.disposition(),
        RunTerminalDisposition::Quarantine {
            quarantine: RunTerminalQuarantine::ExecutionFailure
        }
    );
}

#[test]
fn each_operation_failure_quarantines_both_completed_semantics() {
    for case in [SemanticCase::ScenarioComplete, SemanticCase::HardGateAbort] {
        for failed_index in 0..RUN_TERMINAL_OPERATION_ORDER.len() {
            let intent = terminal_intent(case);
            let outcomes = outcomes_with_failures(&[failed_index]);
            let report = terminal_report(&intent, outcomes);
            let class = RunTerminalClass::classify(&intent, &report).expect("classify report");
            assert_terminal_failure(class);
            assert_eq!(
                report.operations().len(),
                RUN_TERMINAL_OPERATION_ORDER.len()
            );
        }
    }
}

#[test]
fn multiple_failures_keep_all_later_results() {
    let intent = terminal_intent(SemanticCase::ScenarioComplete);
    let report = terminal_report(&intent, outcomes_with_failures(&[0, 3]));
    let class = RunTerminalClass::classify(&intent, &report).expect("classify report");
    assert_terminal_failure(class);
    assert_eq!(
        report
            .operations()
            .last()
            .map(|outcome| outcome.operation()),
        Some(RunTerminalOperation::ChildTerminate)
    );
}

#[test]
fn missing_repeated_extra_and_out_of_order_operations_fail() {
    let intent = terminal_intent(SemanticCase::ScenarioComplete);
    let mut missing = successful_outcomes();
    missing.pop();
    assert!(new_report(&intent, missing).is_err());
    let mut repeated = successful_outcomes();
    repeated[2] = repeated[1].clone();
    assert!(new_report(&intent, repeated).is_err());
    let mut extra = successful_outcomes();
    extra.push(extra[0].clone());
    assert!(new_report(&intent, extra).is_err());
    let mut out_of_order = successful_outcomes();
    out_of_order.swap(0, 1);
    assert!(new_report(&intent, out_of_order).is_err());
}

#[test]
fn child_terminate_success_requires_a_durable_receipt() {
    let intent = terminal_intent(SemanticCase::ScenarioComplete);
    let mut outcomes = successful_outcomes();
    outcomes[5] =
        RunTerminalOperationOutcome::succeeded(RunTerminalOperation::ChildTerminate, None)
            .expect("create unproved success");
    assert!(new_report(&intent, outcomes).is_err());
}

#[test]
fn diagnostic_projection_digest_and_count_tamper_fail() {
    for field in ["projection", "full_digest", "byte_count"] {
        let report = failed_report();
        let mut document = serde_json::to_value(report).expect("encode report");
        change_diagnostic(&mut document, field);
        let changed: RunTerminalReport =
            serde_json::from_value(document).expect("decode changed report");
        assert!(changed.validate().is_err(), "field {field}");
    }
}

#[test]
fn long_diagnostic_keeps_a_bounded_utf8_projection() {
    let detail = "故障".repeat(2_048);
    let diagnostic = RunTerminalDiagnostic::new(&detail).expect("create long diagnostic");
    assert!(diagnostic.projection().len() <= 2_048);
    assert_eq!(diagnostic.byte_count(), detail.len() as u64);
    assert!(!diagnostic.full_digest().is_zero());
    diagnostic.validate().expect("validate long diagnostic");
}

#[test]
fn diagnostic_rejects_shapes_that_new_cannot_make() {
    let diagnostic = RunTerminalDiagnostic::new("short diagnostic").expect("create diagnostic");
    let mut short = serde_json::to_value(&diagnostic).expect("encode diagnostic");
    short["byte_count"] = Value::from(17);
    let changed: RunTerminalDiagnostic =
        serde_json::from_value(short).expect("decode changed diagnostic");
    assert!(changed.validate().is_err());

    let long = RunTerminalDiagnostic::new(&"故障".repeat(2_048)).expect("create long diagnostic");
    let mut truncated = serde_json::to_value(long).expect("encode diagnostic");
    truncated["projection"] = Value::from("too short");
    let changed: RunTerminalDiagnostic =
        serde_json::from_value(truncated).expect("decode changed diagnostic");
    assert!(changed.validate().is_err());
}

#[test]
fn coherent_long_diagnostic_tamper_fails_the_report_digest() {
    let report = long_failed_report();
    let mut document = serde_json::to_value(report).expect("encode report");
    let diagnostic = &mut document["operations"][0]["status"]["diagnostic"];
    let changed_count = diagnostic["byte_count"]
        .as_u64()
        .expect("diagnostic byte count")
        .wrapping_add(1);
    diagnostic["byte_count"] = Value::from(changed_count);
    diagnostic["full_digest"] = serde_json::to_value(fixed_digest(73)).expect("encode full digest");
    let changed_shape: RunTerminalDiagnostic =
        serde_json::from_value(diagnostic.clone()).expect("decode diagnostic");
    changed_shape.validate().expect("validate coherent shape");
    let changed: RunTerminalReport =
        serde_json::from_value(document).expect("decode changed report");
    assert!(changed.validate().is_err());
}

#[test]
fn a_semantic_result_must_match_the_run_context() {
    let context = run_context();
    let mut document = serde_json::to_value(terminal_intent(SemanticCase::ScenarioComplete))
        .expect("encode intent");
    document["outcome"]["run"]["seed"] = Value::from(42);
    let outcome = document["outcome"].clone();
    let changed: RunTerminalSemanticOutcome =
        serde_json::from_value(outcome).expect("decode outcome");
    assert!(
        super::super::RunTerminalIntent::new(
            &context,
            context.digest().expect("digest context"),
            changed,
        )
        .is_err()
    );
}

#[test]
fn semantic_candidate_and_scenario_artifacts_must_match_context() {
    for field in ["candidate_digest", "scenario_digest"] {
        let context = run_context();
        let mut outcome =
            serde_json::to_value(terminal_intent(SemanticCase::ScenarioComplete).outcome())
                .expect("encode outcome");
        outcome[field] = serde_json::to_value(fixed_digest(74)).expect("encode digest");
        let changed: RunTerminalSemanticOutcome =
            serde_json::from_value(outcome).expect("decode changed outcome");
        assert!(new_intent(&context, changed).is_err(), "field {field}");
    }
}

#[test]
fn completed_run_reuses_metric_and_gate_validation() {
    for case in 0..6 {
        let mut run = run_record();
        match case {
            0 => run.loss = -0.1,
            1 => run.control_effort = 1.1,
            2 => {
                run.objectives.insert("tracking".to_owned(), -0.1);
            }
            3 => run.passed_hard_gates.clear(),
            4 => run.passed_hard_gates = vec![" ".to_owned()],
            5 => run.passed_hard_gates = vec!["crash".to_owned(), "crash".to_owned()],
            _ => unreachable!(),
        }
        assert!(scenario_intent(run).is_err(), "case {case}");
    }
}

#[test]
fn hard_gate_abort_rejects_empty_gate_identity_and_detail() {
    for case in 0..2 {
        let mut failure = hard_gate_failure();
        if case == 0 {
            failure.gate.id = " ".to_owned();
        } else {
            failure.gate.detail = String::new();
        }
        assert!(hard_gate_intent(failure).is_err(), "case {case}");
    }
}

#[test]
fn scope_semantic_and_recovery_state_matrix_is_exact() {
    for scope in [
        RunTerminalScope::Active,
        RunTerminalScope::RuntimeOnly,
        RunTerminalScope::NeverStarted,
    ] {
        for state in [
            RunTerminalRecoveryState::Live,
            RunTerminalRecoveryState::Resumed,
        ] {
            for semantic in MatrixSemantic::ALL {
                let result = matrix_report(scope, state, semantic);
                assert_eq!(result.is_ok(), matrix_permits(scope, state, semantic));
            }
        }
    }
}

fn outcomes_with_failures(indices: &[usize]) -> Vec<RunTerminalOperationOutcome> {
    RUN_TERMINAL_OPERATION_ORDER
        .into_iter()
        .enumerate()
        .map(|(index, operation)| {
            if indices.contains(&index) {
                let diagnostic = RunTerminalDiagnostic::new(&format!("failure {index}"))
                    .expect("create operation diagnostic");
                RunTerminalOperationOutcome::failed(operation, diagnostic)
                    .expect("create operation failure")
            } else {
                let proof =
                    (operation == RunTerminalOperation::ChildTerminate).then_some(fixed_digest(90));
                RunTerminalOperationOutcome::succeeded(operation, proof)
                    .expect("create operation success")
            }
        })
        .collect()
}

fn assert_terminal_failure(class: RunTerminalClass) {
    assert_eq!(
        class.disposition(),
        RunTerminalDisposition::Quarantine {
            quarantine: RunTerminalQuarantine::TerminalFailure
        }
    );
}

fn new_report(
    intent: &super::super::RunTerminalIntent,
    outcomes: Vec<RunTerminalOperationOutcome>,
) -> Result<RunTerminalReport, crate::TuneError> {
    RunTerminalReport::new(
        &active_plan(),
        intent,
        RunTerminalRecoveryState::Live,
        outcomes,
    )
}

fn failed_report() -> RunTerminalReport {
    let intent = terminal_intent(SemanticCase::ScenarioComplete);
    terminal_report(&intent, outcomes_with_failures(&[0]))
}

fn long_failed_report() -> RunTerminalReport {
    let intent = terminal_intent(SemanticCase::ScenarioComplete);
    let mut outcomes = successful_outcomes();
    let diagnostic =
        RunTerminalDiagnostic::new(&"terminal failure ".repeat(256)).expect("create diagnostic");
    outcomes[0] =
        RunTerminalOperationOutcome::failed(RunTerminalOperation::SimulatorStop, diagnostic)
            .expect("create failure");
    terminal_report(&intent, outcomes)
}

fn new_intent(
    context: &crate::RunExecutionContext,
    outcome: RunTerminalSemanticOutcome,
) -> Result<RunTerminalIntent, crate::TuneError> {
    RunTerminalIntent::new(
        context,
        context.digest().expect("digest run context"),
        outcome,
    )
}

fn scenario_intent(run: crate::RunRecord) -> Result<RunTerminalIntent, crate::TuneError> {
    let context = run_context();
    let outcome = RunTerminalSemanticOutcome::ScenarioComplete {
        candidate_digest: context.candidate_digest(),
        scenario_digest: context.scenario_digest(),
        run,
    };
    new_intent(&context, outcome)
}

fn hard_gate_intent(
    failure: crate::HardGateFailure,
) -> Result<RunTerminalIntent, crate::TuneError> {
    let context = run_context();
    let outcome = RunTerminalSemanticOutcome::HardGateAbort {
        candidate_digest: context.candidate_digest(),
        scenario_digest: context.scenario_digest(),
        failure,
    };
    new_intent(&context, outcome)
}

#[derive(Clone, Copy)]
enum MatrixSemantic {
    ScenarioComplete,
    HardGateAbort,
    ExecutionError,
    Recovery,
}

impl MatrixSemantic {
    const ALL: [Self; 4] = [
        Self::ScenarioComplete,
        Self::HardGateAbort,
        Self::ExecutionError,
        Self::Recovery,
    ];
}

fn matrix_report(
    scope: RunTerminalScope,
    state: RunTerminalRecoveryState,
    semantic: MatrixSemantic,
) -> Result<RunTerminalReport, crate::TuneError> {
    let plan = RunTerminalPlan::new(scope)?;
    let intent = matrix_intent(semantic);
    RunTerminalReport::new(&plan, &intent, state, outcomes_for_plan(&plan))
}

fn matrix_intent(semantic: MatrixSemantic) -> RunTerminalIntent {
    match semantic {
        MatrixSemantic::ScenarioComplete => terminal_intent(SemanticCase::ScenarioComplete),
        MatrixSemantic::HardGateAbort => terminal_intent(SemanticCase::HardGateAbort),
        MatrixSemantic::ExecutionError => {
            let context = run_context();
            let diagnostic = RunTerminalDiagnostic::new("execution failed").expect("diagnostic");
            new_intent(
                &context,
                RunTerminalSemanticOutcome::ExecutionError { diagnostic },
            )
            .expect("execution intent")
        }
        MatrixSemantic::Recovery => {
            let context = run_context();
            new_intent(&context, RunTerminalSemanticOutcome::Recovery).expect("recovery intent")
        }
    }
}

fn outcomes_for_plan(plan: &RunTerminalPlan) -> Vec<RunTerminalOperationOutcome> {
    plan.requirements()
        .iter()
        .map(|requirement| {
            let operation = requirement.operation();
            if requirement.is_required() {
                let proof =
                    (operation == RunTerminalOperation::ChildTerminate).then_some(fixed_digest(90));
                RunTerminalOperationOutcome::succeeded(operation, proof).expect("success")
            } else {
                RunTerminalOperationOutcome::not_required(operation)
            }
        })
        .collect()
}

const fn matrix_permits(
    scope: RunTerminalScope,
    state: RunTerminalRecoveryState,
    semantic: MatrixSemantic,
) -> bool {
    match semantic {
        MatrixSemantic::ScenarioComplete | MatrixSemantic::HardGateAbort => {
            matches!(scope, RunTerminalScope::Active)
        }
        MatrixSemantic::ExecutionError => true,
        MatrixSemantic::Recovery => matches!(state, RunTerminalRecoveryState::Resumed),
    }
}

fn change_diagnostic(document: &mut Value, field: &str) {
    let diagnostic = &mut document["operations"][0]["status"]["diagnostic"];
    match field {
        "projection" => diagnostic[field] = Value::from("changed"),
        "full_digest" => {
            diagnostic[field] = serde_json::to_value(fixed_digest(77)).expect("encode digest");
        }
        "byte_count" => diagnostic[field] = Value::from(0),
        _ => panic!("unknown test field"),
    }
}
