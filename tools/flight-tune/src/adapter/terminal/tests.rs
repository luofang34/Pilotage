#![allow(clippy::expect_used)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::*;
use crate::terminal::{
    RunTerminalClass, RunTerminalOperationOutcome, RunTerminalRecoveryState, RunTerminalReport,
    RunTerminalSemanticOutcome,
};
use crate::{
    ArtifactIdentity, AttemptRole, RunExecutionContext, ScenarioRef, ScenarioSet,
    SimulatorSessionReceipt, VehicleBindingReceipt,
};

struct FakeTerminalAdapter {
    capabilities: RunTerminalCapabilities,
    calls: Rc<Cell<usize>>,
    recovered: Rc<RefCell<Vec<RunTerminalReceipt>>>,
    child_proof: Digest,
}

impl RunTerminalAdapter for FakeTerminalAdapter {
    fn terminal_capabilities(&self) -> RunTerminalCapabilities {
        self.capabilities
    }

    fn bind_terminal_plan_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _binding: &RunBindingReceipt,
        _plan: &RunTerminalPlan,
    ) -> Result<(), AdapterError> {
        self.record_call();
        Ok(())
    }

    fn child_terminate_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _binding: &RunBindingReceipt,
    ) -> Result<Digest, AdapterError> {
        self.record_call();
        Ok(self.child_proof)
    }

    fn seal_terminal_receipt_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _binding: &RunBindingReceipt,
        _receipt: &RunTerminalReceipt,
    ) -> Result<(), AdapterError> {
        self.record_call();
        Ok(())
    }

    fn recover_terminal_receipts_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _binding: &RunBindingReceipt,
    ) -> Result<Vec<RunTerminalReceipt>, AdapterError> {
        self.record_call();
        Ok(self.recovered.borrow().clone())
    }
}

impl FakeTerminalAdapter {
    fn record_call(&self) {
        self.calls.set(self.calls.get().wrapping_add(1));
    }
}

struct Fixture {
    capability: SimulatorCapability,
    plan: RunTerminalPlan,
    binding: RunBindingReceipt,
    vehicle: VehicleBinding<FakeTerminalAdapter>,
    calls: Rc<Cell<usize>>,
    recovered: Rc<RefCell<Vec<RunTerminalReceipt>>>,
}

#[test]
fn capabilities_make_the_exact_scope_plans() {
    let none = RunTerminalCapabilities::new(false, false, false);
    assert_requirements(
        &none
            .plan_for_scope(RunTerminalScope::Active)
            .expect("make active plan"),
        [true, false, false, false, false, false],
    );
    assert_requirements(
        &none
            .plan_for_scope(RunTerminalScope::RuntimeOnly)
            .expect("make runtime plan"),
        [false; 6],
    );

    let all = RunTerminalCapabilities::new(true, true, true);
    assert_requirements(
        &all.plan_for_scope(RunTerminalScope::Active)
            .expect("make active plan"),
        [true; 6],
    );
    assert_requirements(
        &all.plan_for_scope(RunTerminalScope::RuntimeOnly)
            .expect("make runtime plan"),
        [false, true, true, true, true, true],
    );
    assert_requirements(
        &all.plan_for_scope(RunTerminalScope::NeverStarted)
            .expect("make idle plan"),
        [false; 6],
    );

    let trace_and_child = RunTerminalCapabilities::new(false, true, true);
    assert_requirements(
        &trace_and_child
            .plan_for_scope(RunTerminalScope::Active)
            .expect("make partial plan"),
        [true, false, true, true, true, true],
    );
}

#[test]
fn mismatched_session_vehicle_and_plan_stop_before_the_adapter_call() {
    let mut fixture = fixture(fixed_digest(6));
    let foreign_capability = simulator_capability(fixed_digest(31));
    assert!(
        fixture
            .vehicle
            .bind_terminal_plan_blocking(&foreign_capability, &fixture.binding, &fixture.plan,)
            .is_err()
    );

    let foreign_vehicle = run_binding(fixture.binding.context(), &fixture.plan, fixed_digest(32));
    assert!(
        fixture
            .vehicle
            .bind_terminal_plan_blocking(&fixture.capability, &foreign_vehicle, &fixture.plan,)
            .is_err()
    );

    let foreign_plan = RunTerminalCapabilities::new(false, false, false)
        .plan_for_scope(RunTerminalScope::Active)
        .expect("make foreign plan");
    assert!(
        fixture
            .vehicle
            .bind_terminal_plan_blocking(&fixture.capability, &fixture.binding, &foreign_plan,)
            .is_err()
    );
    assert_eq!(fixture.calls.get(), 0);
}

#[test]
fn a_zero_child_termination_proof_is_rejected() {
    let mut fixture = fixture(Digest::from_bytes([0; 32]));
    assert!(
        fixture
            .vehicle
            .terminal_child_terminate_blocking(
                &fixture.capability,
                &fixture.binding,
                &fixture.plan,
            )
            .is_err()
    );
    assert_eq!(fixture.calls.get(), 1);
}

#[test]
fn recovery_returns_raw_receipts_for_core_validation() {
    let mut foreign = fixture(fixed_digest(6));
    let foreign_binding = run_binding(foreign.binding.context(), &foreign.plan, fixed_digest(44));
    foreign
        .recovered
        .borrow_mut()
        .push(terminal_receipt(&foreign_binding, &foreign.plan));
    let foreign_receipts = foreign
        .vehicle
        .recover_terminal_receipts_blocking(&foreign.capability, &foreign.binding, &foreign.plan)
        .expect("return foreign receipt for core validation");
    assert_ne!(foreign_receipts[0].binding(), &foreign.binding);
    assert_eq!(foreign.calls.get(), 1);

    let mut changed = fixture(fixed_digest(6));
    let receipt = terminal_receipt(&changed.binding, &changed.plan);
    changed
        .recovered
        .borrow_mut()
        .push(changed_receipt_digest(receipt));
    let changed_receipts = changed
        .vehicle
        .recover_terminal_receipts_blocking(&changed.capability, &changed.binding, &changed.plan)
        .expect("return malformed receipt for core validation");
    assert!(changed_receipts[0].validate().is_err());
    assert_eq!(changed.calls.get(), 1);
}

#[test]
fn sealing_rejects_a_foreign_receipt_before_the_adapter_call() {
    let mut fixture = fixture(fixed_digest(6));
    let foreign_binding = run_binding(fixture.binding.context(), &fixture.plan, fixed_digest(45));
    let receipt = terminal_receipt(&foreign_binding, &fixture.plan);
    assert!(
        fixture
            .vehicle
            .seal_terminal_receipt_blocking(
                &fixture.capability,
                &fixture.binding,
                &fixture.plan,
                &receipt,
            )
            .is_err()
    );
    assert_eq!(fixture.calls.get(), 0);
}

fn fixture(child_proof: Digest) -> Fixture {
    let session_digest = fixed_digest(1);
    let vehicle_digest = fixed_digest(5);
    let capability = simulator_capability(session_digest);
    let capabilities = RunTerminalCapabilities::new(true, true, true);
    let plan = capabilities
        .plan_for_scope(RunTerminalScope::Active)
        .expect("make terminal plan");
    let binding = run_binding(&run_context(session_digest), &plan, vehicle_digest);
    let calls = Rc::new(Cell::new(0));
    let recovered = Rc::new(RefCell::new(Vec::new()));
    let adapter = FakeTerminalAdapter {
        capabilities,
        calls: Rc::clone(&calls),
        recovered: Rc::clone(&recovered),
        child_proof,
    };
    let vehicle = capability
        .bind_vehicle(
            adapter,
            VehicleBindingReceipt {
                session_digest,
                vehicle_digest,
            },
        )
        .expect("bind vehicle");
    Fixture {
        capability,
        plan,
        binding,
        vehicle,
        calls,
        recovered,
    }
}

fn terminal_receipt(binding: &RunBindingReceipt, plan: &RunTerminalPlan) -> RunTerminalReceipt {
    let context = binding.context();
    let intent = crate::terminal::RunTerminalIntent::new(
        context,
        context.digest().expect("digest context"),
        RunTerminalSemanticOutcome::Recovery,
    )
    .expect("make recovery intent");
    let operations = plan
        .requirements()
        .iter()
        .copied()
        .map(operation_outcome)
        .collect::<Vec<_>>();
    let report =
        RunTerminalReport::new(plan, &intent, RunTerminalRecoveryState::Resumed, operations)
            .expect("make terminal report");
    let class = RunTerminalClass::classify(&intent, &report).expect("classify report");
    RunTerminalReceipt::new(binding, &intent, &report, class, fixed_digest(70))
        .expect("make terminal receipt")
}

fn operation_outcome(requirement: RunTerminalRequirement) -> RunTerminalOperationOutcome {
    let operation = requirement.operation();
    if !requirement.is_required() {
        return RunTerminalOperationOutcome::not_required(operation);
    }
    let proof = (operation == RunTerminalOperation::ChildTerminate).then_some(fixed_digest(71));
    RunTerminalOperationOutcome::succeeded(operation, proof).expect("make operation result")
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
    let scenario = ScenarioRef {
        id: "terminal-reference".to_owned(),
        digest: fixed_digest(3),
        max_samples: 10,
        sample_timeout_ms: 20,
    };
    RunExecutionContext::new(
        session_digest,
        1,
        AttemptRole::TrainingBaseline,
        fixed_digest(4),
        None,
        ScenarioSet::Training,
        &scenario,
        0,
        41,
    )
    .expect("make run context")
}

fn simulator_capability(session_digest: Digest) -> SimulatorCapability {
    SimulatorCapability::new(SimulatorSessionReceipt {
        session_digest,
        simulator_digest: fixed_digest(2),
        airframe_digest: fixed_digest(3),
    })
}

fn assert_requirements(plan: &RunTerminalPlan, expected: [bool; 6]) {
    assert_eq!(plan.requirements().len(), expected.len());
    for (requirement, expected_required) in plan.requirements().iter().zip(expected) {
        assert_eq!(requirement.is_required(), expected_required);
    }
}

fn changed_receipt_digest(receipt: RunTerminalReceipt) -> RunTerminalReceipt {
    let mut document = serde_json::to_value(receipt).expect("encode receipt");
    document["receipt_digest"] = serde_json::to_value(fixed_digest(99)).expect("encode digest");
    serde_json::from_value::<RunTerminalReceipt>(document).expect("decode changed receipt")
}

fn fixed_digest(value: u8) -> Digest {
    Digest::from_bytes([value; 32])
}
