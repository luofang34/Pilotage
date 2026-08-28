use serde_json::Value;

use crate::{
    AttemptRole, CandidateTransitionReference, MissionReference, RunExecutionContext, ScenarioSet,
};

use super::{
    SemanticCase, active_plan, binding, fixed_digest, successful_outcomes, terminal_intent,
    terminal_receipt, terminal_report,
};
use crate::terminal::{RunBindingReceipt, RunTerminalIntent, RunTerminalPlan};
use crate::terminal::{RunTerminalOperation, RunTerminalRequirement, RunTerminalScope};

#[test]
fn every_saved_document_recomputes_its_canonical_digest() {
    let plan = active_plan();
    let intent = terminal_intent(SemanticCase::ScenarioComplete);
    let binding = binding(&intent);
    let report = terminal_report(&intent, successful_outcomes());
    let receipt = terminal_receipt(SemanticCase::ScenarioComplete);
    assert_eq!(
        plan.plan_digest(),
        plan.recompute_digest().expect("plan digest")
    );
    assert_eq!(
        binding.receipt_digest(),
        binding.recompute_digest().expect("binding digest")
    );
    assert_eq!(
        intent.intent_digest(),
        intent.recompute_digest().expect("intent digest")
    );
    assert_eq!(
        report.report_digest(),
        report.recompute_digest().expect("report digest")
    );
    assert_eq!(
        receipt.receipt_digest(),
        receipt.recompute_digest().expect("receipt digest")
    );
}

#[test]
fn changed_plan_order_fails_validation() {
    let mut document = serde_json::to_value(active_plan()).expect("encode plan");
    document["requirements"]
        .as_array_mut()
        .expect("requirements array")
        .swap(0, 1);
    let changed: RunTerminalPlan = serde_json::from_value(document).expect("decode plan");
    assert!(changed.validate().is_err());
}

#[test]
fn changed_binding_adapter_fails_validation() {
    let intent = terminal_intent(SemanticCase::ScenarioComplete);
    let mut document = serde_json::to_value(binding(&intent)).expect("encode binding");
    document["adapter"]["digest"] = serde_json::to_value(fixed_digest(61)).expect("encode digest");
    let changed: RunBindingReceipt = serde_json::from_value(document).expect("decode binding");
    assert!(changed.validate().is_err());
}

#[test]
fn changed_semantic_intent_fails_validation() {
    let mut document = serde_json::to_value(terminal_intent(SemanticCase::ScenarioComplete))
        .expect("encode intent");
    document["outcome"]["run"]["loss"] = Value::from(0.7);
    let changed: RunTerminalIntent = serde_json::from_value(document).expect("decode intent");
    assert!(changed.validate().is_err());
}

#[test]
fn changed_run_context_and_transition_reference_fail_validation() {
    let original = terminal_intent(SemanticCase::ScenarioComplete);
    let mut context_changed = serde_json::to_value(&original).expect("encode intent");
    context_changed["context"]["candidate_digest"] =
        serde_json::to_value(fixed_digest(62)).expect("encode candidate digest");
    let changed: RunTerminalIntent =
        serde_json::from_value(context_changed).expect("decode changed context");
    assert!(changed.validate().is_err());

    let mut transition_changed =
        serde_json::to_value(challenger_intent()).expect("encode challenger intent");
    transition_changed["context"]["transition_authorization"]["receipt_digest"] =
        serde_json::to_value(fixed_digest(8)).expect("encode receipt digest");
    let changed: RunTerminalIntent =
        serde_json::from_value(transition_changed).expect("decode changed transition");
    changed
        .context()
        .validate()
        .expect("changed transition remains structurally valid");
    assert!(changed.validate().is_err());
}

#[test]
fn core_can_freeze_a_capability_specific_prestart_plan() {
    let active = RunTerminalPlan::from_requirements(
        RunTerminalScope::Active,
        [
            required(RunTerminalOperation::SimulatorStop),
            not_required(RunTerminalOperation::ControlStop),
            required(RunTerminalOperation::TraceStop),
            not_required(RunTerminalOperation::ChildHealth),
            required(RunTerminalOperation::TraceShutdown),
            required(RunTerminalOperation::ChildTerminate),
        ],
    )
    .expect("create active capability plan");
    assert!(!active.requirements()[1].is_required());
    assert!(!active.requirements()[3].is_required());

    let runtime = RunTerminalPlan::from_requirements(
        RunTerminalScope::RuntimeOnly,
        [
            not_required(RunTerminalOperation::SimulatorStop),
            not_required(RunTerminalOperation::ControlStop),
            required(RunTerminalOperation::TraceStop),
            required(RunTerminalOperation::ChildHealth),
            not_required(RunTerminalOperation::TraceShutdown),
            required(RunTerminalOperation::ChildTerminate),
        ],
    )
    .expect("create runtime capability plan");
    runtime.validate().expect("validate runtime plan");
}

#[test]
fn capability_plan_enforces_scope_and_fixed_order() {
    let all_not_required = [
        not_required(RunTerminalOperation::SimulatorStop),
        not_required(RunTerminalOperation::ControlStop),
        not_required(RunTerminalOperation::TraceStop),
        not_required(RunTerminalOperation::ChildHealth),
        not_required(RunTerminalOperation::TraceShutdown),
        not_required(RunTerminalOperation::ChildTerminate),
    ];
    assert!(
        RunTerminalPlan::from_requirements(RunTerminalScope::Active, all_not_required).is_err()
    );
    let mut runtime = all_not_required;
    runtime[0] = required(RunTerminalOperation::SimulatorStop);
    assert!(RunTerminalPlan::from_requirements(RunTerminalScope::RuntimeOnly, runtime).is_err());
    let mut never = all_not_required;
    never[4] = required(RunTerminalOperation::TraceShutdown);
    assert!(RunTerminalPlan::from_requirements(RunTerminalScope::NeverStarted, never).is_err());
    let mut repeated = all_not_required;
    repeated[2] = not_required(RunTerminalOperation::ControlStop);
    assert!(RunTerminalPlan::from_requirements(RunTerminalScope::NeverStarted, repeated).is_err());
}

const fn required(operation: RunTerminalOperation) -> RunTerminalRequirement {
    RunTerminalRequirement::Required { operation }
}

const fn not_required(operation: RunTerminalOperation) -> RunTerminalRequirement {
    RunTerminalRequirement::NotRequired { operation }
}

fn challenger_intent() -> RunTerminalIntent {
    let transition: CandidateTransitionReference = serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "session_digest": fixed_digest(1),
        "source_candidate_digest": fixed_digest(2),
        "target_candidate_digest": fixed_digest(3),
        "validator_digest": fixed_digest(4),
        "adjacency_policy_digest": fixed_digest(5),
        "planning_context_digest": fixed_digest(6),
        "receipt_digest": fixed_digest(7),
    }))
    .expect("decode transition reference");
    let scenario = MissionReference {
        revision_id: "step-calm".to_owned(),
        schema_version: flight_tune::MISSION_SCHEMA_VERSION,
        content_digest: fixed_digest(2),
        max_samples: 100,
        sample_timeout_ns: 20_000_000,
    };
    let context = RunExecutionContext::new(
        fixed_digest(1),
        7,
        AttemptRole::TrainingChallenger { attempt_index: 0 },
        fixed_digest(3),
        Some(transition),
        ScenarioSet::Training,
        &scenario,
        0,
        41,
    )
    .expect("create challenger context");
    RunTerminalIntent::new(
        &context,
        context.digest().expect("digest challenger context"),
        crate::terminal::RunTerminalSemanticOutcome::ScenarioComplete {
            candidate_digest: context.candidate_digest(),
            mission_content_digest: context.mission_content_digest(),
            run: super::run_record(),
        },
    )
    .expect("create challenger intent")
}
