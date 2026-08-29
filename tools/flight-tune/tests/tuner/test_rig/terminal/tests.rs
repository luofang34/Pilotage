use std::collections::BTreeMap;

use flight_tune::{
    ArtifactIdentity, AttemptRole, Digest, MissionReference, RunBindingReceipt,
    RunExecutionContext, RunRecord, RunTerminalClass, RunTerminalOperation,
    RunTerminalOperationOutcome, RunTerminalPlan, RunTerminalReceipt, RunTerminalRecoveryState,
    RunTerminalReport, RunTerminalScope, RunTerminalSemanticOutcome, ScenarioSet,
};

use super::super::terminal_state::{
    FakeTerminalReadbackFault, FakeTerminalSealFault, FakeTerminalState,
};

#[test]
fn terminal_state_binds_idempotently_and_records_all_later_operations() {
    let (binding, plan) = binding_and_plan(fixed_digest(1), fixed_digest(5));
    let mut state = FakeTerminalState::default();
    assert!(state.capabilities.has_control());
    assert!(state.capabilities.has_trace());
    assert!(state.capabilities.has_supervised_child());

    state.bind_plan(&binding, &plan).expect("bind plan");
    state.bind_plan(&binding, &plan).expect("repeat plan bind");
    assert_eq!(state.bind_count(), 2);
    assert_eq!(state.bound_plan_count(), 1);

    state.failed_operations = vec![
        RunTerminalOperation::ControlStop,
        RunTerminalOperation::ChildHealth,
    ];
    let operations = [
        RunTerminalOperation::ControlStop,
        RunTerminalOperation::TraceStop,
        RunTerminalOperation::ChildHealth,
        RunTerminalOperation::TraceShutdown,
        RunTerminalOperation::ChildTerminate,
    ];
    for operation in operations {
        let result = state.run_operation(&binding, operation);
        assert_eq!(
            result.is_err(),
            state.failed_operations.contains(&operation),
            "operation {operation:?}",
        );
    }

    assert_eq!(
        state.operation_count(RunTerminalOperation::SimulatorStop),
        0
    );
    for operation in operations {
        assert_eq!(state.operation_count(operation), 1);
    }
    assert_eq!(state.operation_order(), operations);
    assert_eq!(
        state
            .run_operation(&binding, RunTerminalOperation::ChildTerminate)
            .expect("repeat child termination"),
        Some(state.child_terminate_proof),
    );

    assert_eq!(
        state
            .read_causal_evidence(&binding)
            .expect("read causal evidence"),
        state.causal_evidence_digest,
    );
    assert_eq!(state.causal_evidence_read_count(), 1);
}

#[test]
fn seal_faults_are_one_shot_and_the_exact_store_is_idempotent() {
    let (binding, plan) = binding_and_plan(fixed_digest(1), fixed_digest(5));
    let receipt = recovery_receipt(&binding, &plan);
    let mut state = bound_state(&binding, &plan);

    state.seal_fault = FakeTerminalSealFault::FailBeforePublication;
    assert!(state.seal_receipt(&binding, &receipt).is_err());
    assert_eq!(state.seal_fault, FakeTerminalSealFault::None);
    assert!(state.receipts(binding.receipt_digest()).is_empty());

    state
        .seal_receipt(&binding, &receipt)
        .expect("seal quarantine after the one-shot failure");
    state
        .seal_receipt(&binding, &receipt)
        .expect("repeat exact seal");
    assert_eq!(state.seal_count(), 3);
    assert_eq!(
        state.receipts(binding.receipt_digest()),
        std::slice::from_ref(&receipt)
    );

    let recovered = state
        .recover_receipts(&binding)
        .expect("recover exact receipt");
    assert_eq!(recovered, [receipt]);
    assert_eq!(state.recover_count(), 1);
}

#[test]
fn lost_ack_and_all_recovery_injections_are_observable() {
    let (binding, plan) = binding_and_plan(fixed_digest(1), fixed_digest(5));
    let quarantine = recovery_receipt(&binding, &plan);
    let completed = completed_receipt(&binding, &plan);
    let mut state = bound_state(&binding, &plan);

    state.seal_fault = FakeTerminalSealFault::LoseAcknowledgement;
    assert!(state.seal_receipt(&binding, &quarantine).is_err());
    assert_eq!(state.seal_fault, FakeTerminalSealFault::None);
    assert_eq!(
        state.receipts(binding.receipt_digest()),
        std::slice::from_ref(&quarantine)
    );
    state
        .seal_receipt(&binding, &quarantine)
        .expect("repeat seal after lost acknowledgement");

    let foreign_binding = run_binding(binding.context(), &plan, fixed_digest(44));
    let foreign = recovery_receipt(&foreign_binding, &plan);
    let changed = changed_receipt(quarantine.clone());
    let injections = [
        vec![quarantine.clone()],
        vec![changed],
        vec![foreign.clone()],
        vec![quarantine.clone(), foreign],
        vec![completed.clone(), quarantine],
    ];
    for receipts in injections {
        state.recovery_receipts = Some(receipts.clone());
        assert_eq!(
            state
                .recover_receipts(&binding)
                .expect("return injected receipts"),
            receipts,
        );
    }
    assert_eq!(state.recover_count(), 5);

    state.recovery_receipts = Some(vec![completed.clone()]);
    state.readback_fault = FakeTerminalReadbackFault::ChangeReceipt;
    let changed = state
        .recover_receipts(&binding)
        .expect("return changed readback");
    assert_ne!(changed, [completed]);
    assert_eq!(state.readback_fault, FakeTerminalReadbackFault::None);

    state.readback_fault = FakeTerminalReadbackFault::TwoClasses;
    let two_classes = state
        .recover_receipts(&binding)
        .expect("return two receipt classes");
    assert_eq!(two_classes.len(), 2);
    assert_ne!(two_classes[0].is_completed(), two_classes[1].is_completed());
    assert_eq!(state.readback_fault, FakeTerminalReadbackFault::None);
}

fn bound_state(binding: &RunBindingReceipt, plan: &RunTerminalPlan) -> FakeTerminalState {
    let mut state = FakeTerminalState::default();
    state.bind_plan(binding, plan).expect("bind terminal plan");
    state
}

fn binding_and_plan(
    session_digest: Digest,
    vehicle_digest: Digest,
) -> (RunBindingReceipt, RunTerminalPlan) {
    let plan = RunTerminalPlan::new(RunTerminalScope::Active).expect("make terminal plan");
    let context = run_context(session_digest);
    let binding = run_binding(&context, &plan, vehicle_digest);
    (binding, plan)
}

fn run_binding(
    context: &RunExecutionContext,
    plan: &RunTerminalPlan,
    vehicle_digest: Digest,
) -> RunBindingReceipt {
    let adapter =
        ArtifactIdentity::new("terminal-adapter", vehicle_digest).expect("make adapter identity");
    RunBindingReceipt::new(context, plan, adapter).expect("make run binding")
}

fn run_context(session_digest: Digest) -> RunExecutionContext {
    let scenario = MissionReference {
        revision_id: "terminal-reference".to_owned(),
        schema_version: flight_tune::MISSION_SCHEMA_VERSION,
        content_digest: fixed_digest(3),
        max_samples: 10,
        sample_timeout_ns: 20_000_000,
    };
    RunExecutionContext::new(
        session_digest,
        1,
        AttemptRole::TrainingBaseline { suite_index: 0 },
        fixed_digest(4),
        None,
        ScenarioSet::Training,
        &scenario,
        0,
        41,
        0,
    )
    .expect("make run context")
}

fn recovery_receipt(binding: &RunBindingReceipt, plan: &RunTerminalPlan) -> RunTerminalReceipt {
    make_receipt(
        binding,
        plan,
        RunTerminalSemanticOutcome::Recovery,
        RunTerminalRecoveryState::Resumed,
    )
}

fn completed_receipt(binding: &RunBindingReceipt, plan: &RunTerminalPlan) -> RunTerminalReceipt {
    let context = binding.context();
    let run = RunRecord {
        scenario_set: context.scenario_set(),
        mission_revision_id: context.mission_revision_id().to_owned(),
        repetition: context.repetition(),
        seed: context.seed(),
        loss: 0.2,
        control_effort: 0.3,
        objectives: BTreeMap::from([("tracking".to_owned(), 0.2)]),
        passed_hard_gates: vec!["crash".to_owned()],
    };
    make_receipt(
        binding,
        plan,
        RunTerminalSemanticOutcome::ScenarioComplete {
            candidate_digest: context.candidate_digest(),
            mission_content_digest: context.mission_content_digest(),
            run,
        },
        RunTerminalRecoveryState::Live,
    )
}

fn make_receipt(
    binding: &RunBindingReceipt,
    plan: &RunTerminalPlan,
    outcome: RunTerminalSemanticOutcome,
    recovery_state: RunTerminalRecoveryState,
) -> RunTerminalReceipt {
    let context = binding.context();
    let intent = flight_tune::RunTerminalIntent::new(
        context,
        context.digest().expect("digest context"),
        outcome,
    )
    .expect("make terminal intent");
    let operations = plan
        .requirements()
        .iter()
        .copied()
        .map(operation_outcome)
        .collect::<Vec<_>>();
    let report = RunTerminalReport::new(plan, &intent, recovery_state, operations)
        .expect("make terminal report");
    let class = RunTerminalClass::classify(&intent, &report).expect("classify terminal report");
    RunTerminalReceipt::new(binding, &intent, &report, class, fixed_digest(70))
        .expect("make terminal receipt")
}

fn operation_outcome(
    requirement: flight_tune::RunTerminalRequirement,
) -> RunTerminalOperationOutcome {
    let operation = requirement.operation();
    if !requirement.is_required() {
        return RunTerminalOperationOutcome::not_required(operation);
    }
    let proof = (operation == RunTerminalOperation::ChildTerminate).then_some(fixed_digest(71));
    RunTerminalOperationOutcome::succeeded(operation, proof).expect("make operation result")
}

fn changed_receipt(receipt: RunTerminalReceipt) -> RunTerminalReceipt {
    let mut document = serde_json::to_value(receipt).expect("encode receipt");
    document["receipt_digest"] = serde_json::to_value(fixed_digest(99)).expect("encode digest");
    serde_json::from_value(document).expect("decode changed receipt")
}

const fn fixed_digest(value: u8) -> Digest {
    Digest::from_bytes([value; 32])
}
