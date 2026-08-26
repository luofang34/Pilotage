use std::collections::HashMap;
use std::path::Path;

use flight_tune::{
    AdapterError, Digest, RunBindingReceipt, RunTerminalCapabilities, RunTerminalClass,
    RunTerminalOperation, RunTerminalPlan, RunTerminalReceipt,
};

use super::terminal_head_poison::{FakeTerminalHeadPoison, TerminalExternalAction};

/// One terminal receipt publication fault for the reference adapter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FakeTerminalSealFault {
    /// Receipt publication and acknowledgement succeed.
    #[default]
    None,
    /// Publication fails before the receipt becomes visible.
    FailBeforePublication,
    /// Publication reports success without making a receipt durable.
    SucceedWithoutPublication,
    /// The exact receipt becomes visible before acknowledgement fails.
    LoseAcknowledgement,
}

/// One terminal receipt readback fault for the reference adapter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FakeTerminalReadbackFault {
    /// The adapter returns its exact durable receipt set.
    #[default]
    None,
    /// The adapter returns one valid receipt with changed evidence identity.
    ChangeReceipt,
    /// The adapter returns both completed and quarantine receipt classes.
    TwoClasses,
    /// The adapter returns one transient read error.
    ReturnError,
}

#[derive(Debug, Clone)]
struct BoundTerminalPlan {
    binding: RunBindingReceipt,
    plan: RunTerminalPlan,
}

/// Configurable in-memory terminal state for the reference adapter.
#[derive(Debug)]
pub struct FakeTerminalState {
    /// The fixed component capabilities returned by the adapter.
    pub capabilities: RunTerminalCapabilities,
    /// Operations that return an injected failure after they are recorded.
    pub failed_operations: Vec<RunTerminalOperation>,
    /// The next plan bind becomes durable before its acknowledgement fails.
    pub lose_bind_acknowledgement: bool,
    /// The configured receipt publication fault.
    pub seal_fault: FakeTerminalSealFault,
    /// Optional receipt readback that replaces the exact in-memory store.
    ///
    /// Tests can supply changed, foreign, conflicting, or two-class receipts.
    pub recovery_receipts: Option<Vec<RunTerminalReceipt>>,
    /// The next nonempty durable readback fault.
    pub readback_fault: FakeTerminalReadbackFault,
    /// The stable causal evidence identity.
    pub causal_evidence_digest: Digest,
    /// The stable supervised-child termination proof.
    pub child_terminate_proof: Digest,
    /// The simulator stop operation returns an injected failure when this is true.
    pub fail_simulator_stop: bool,
    head_poison: FakeTerminalHeadPoison,
    capabilities_read_count: usize,
    bound_plans: HashMap<Digest, BoundTerminalPlan>,
    receipts: HashMap<Digest, Vec<RunTerminalReceipt>>,
    operation_counts: [usize; 6],
    operation_order: Vec<RunTerminalOperation>,
    bind_count: usize,
    causal_evidence_read_count: usize,
    seal_count: usize,
    recover_count: usize,
}

impl Default for FakeTerminalState {
    fn default() -> Self {
        Self {
            capabilities: RunTerminalCapabilities::new(true, true, true),
            failed_operations: Vec::new(),
            lose_bind_acknowledgement: false,
            seal_fault: FakeTerminalSealFault::None,
            recovery_receipts: None,
            readback_fault: FakeTerminalReadbackFault::None,
            causal_evidence_digest: stable_digest(81),
            child_terminate_proof: stable_digest(82),
            fail_simulator_stop: false,
            head_poison: FakeTerminalHeadPoison::default(),
            capabilities_read_count: 0,
            bound_plans: HashMap::new(),
            receipts: HashMap::new(),
            operation_counts: [0; 6],
            operation_order: Vec::new(),
            bind_count: 0,
            causal_evidence_read_count: 0,
            seal_count: 0,
            recover_count: 0,
        }
    }
}

impl FakeTerminalState {
    /// Changes the journal head after one terminal authority boundary.
    pub fn poison_head_after(&mut self, action: TerminalExternalAction, root: &Path) {
        self.head_poison.arm(action, root);
    }

    pub(super) fn take_head_poison(
        &mut self,
        action: TerminalExternalAction,
    ) -> Option<std::path::PathBuf> {
        self.head_poison.take(action)
    }

    /// Returns the number of terminal capability reads.
    #[must_use]
    pub const fn capabilities_read_count(&self) -> usize {
        self.capabilities_read_count
    }

    pub(super) fn read_capabilities(
        &mut self,
    ) -> (RunTerminalCapabilities, Option<std::path::PathBuf>) {
        self.capabilities_read_count = self.capabilities_read_count.wrapping_add(1);
        let head_poison = self.take_head_poison(TerminalExternalAction::PlanRead);
        (self.capabilities, head_poison)
    }

    /// Returns the number of exact plan bind calls.
    #[must_use]
    pub const fn bind_count(&self) -> usize {
        self.bind_count
    }

    /// Returns the number of calls for one terminal operation.
    #[must_use]
    pub fn operation_count(&self, operation: RunTerminalOperation) -> usize {
        self.operation_counts[operation_index(operation)]
    }

    /// Returns adapter-owned operations in their call order.
    #[must_use]
    pub fn operation_order(&self) -> &[RunTerminalOperation] {
        &self.operation_order
    }

    /// Returns the number of causal evidence reads.
    #[must_use]
    pub const fn causal_evidence_read_count(&self) -> usize {
        self.causal_evidence_read_count
    }

    /// Returns the number of terminal receipt seal calls.
    #[must_use]
    pub const fn seal_count(&self) -> usize {
        self.seal_count
    }

    /// Returns the number of terminal receipt recovery calls.
    #[must_use]
    pub const fn recover_count(&self) -> usize {
        self.recover_count
    }

    /// Returns the number of exact immutable plan bindings.
    #[must_use]
    pub fn bound_plan_count(&self) -> usize {
        self.bound_plans.len()
    }

    /// Returns the saved receipts for one binding digest.
    #[must_use]
    pub fn receipts(&self, binding_digest: Digest) -> &[RunTerminalReceipt] {
        self.receipts
            .get(&binding_digest)
            .map_or(&[], Vec::as_slice)
    }

    pub(super) fn bind_plan(
        &mut self,
        binding: &RunBindingReceipt,
        plan: &RunTerminalPlan,
    ) -> Result<(), AdapterError> {
        validate_binding_plan(binding, plan)?;
        self.bind_count = self.bind_count.wrapping_add(1);
        let key = binding.receipt_digest();
        if let Some(saved) = self.bound_plans.get(&key) {
            if saved.binding != *binding || saved.plan != *plan {
                return Err(AdapterError::new(
                    "the terminal plan binding changed after publication",
                ));
            }
        } else {
            self.bound_plans.insert(
                key,
                BoundTerminalPlan {
                    binding: binding.clone(),
                    plan: plan.clone(),
                },
            );
        }
        if std::mem::take(&mut self.lose_bind_acknowledgement) {
            return Err(AdapterError::new(
                "terminal plan bind acknowledgement was lost",
            ));
        }
        Ok(())
    }

    pub(super) fn run_operation(
        &mut self,
        binding: &RunBindingReceipt,
        operation: RunTerminalOperation,
    ) -> Result<Option<Digest>, AdapterError> {
        self.require_exact_binding(binding)?;
        let index = operation_index(operation);
        self.operation_counts[index] = self.operation_counts[index].wrapping_add(1);
        self.operation_order.push(operation);
        if self.failed_operations.contains(&operation) {
            return Err(AdapterError::new(format!(
                "the reference terminal adapter failed {operation:?}",
            )));
        }
        let proof = if operation == RunTerminalOperation::ChildTerminate {
            self.child_terminate_proof
        } else {
            stable_digest(90_u8.wrapping_add(index as u8))
        };
        Ok(Some(proof))
    }

    pub(super) fn read_causal_evidence(
        &mut self,
        binding: &RunBindingReceipt,
    ) -> Result<Digest, AdapterError> {
        self.require_exact_binding(binding)?;
        self.causal_evidence_read_count = self.causal_evidence_read_count.wrapping_add(1);
        Ok(self.causal_evidence_digest)
    }

    pub(super) fn seal_receipt(
        &mut self,
        binding: &RunBindingReceipt,
        receipt: &RunTerminalReceipt,
    ) -> Result<(), AdapterError> {
        self.require_exact_binding(binding)?;
        validate_exact_receipt(binding, receipt)?;
        self.seal_count = self.seal_count.wrapping_add(1);
        let seal_fault = std::mem::take(&mut self.seal_fault);
        if seal_fault == FakeTerminalSealFault::FailBeforePublication {
            return Err(AdapterError::new(
                "terminal receipt publication failed before publication",
            ));
        }
        if seal_fault == FakeTerminalSealFault::SucceedWithoutPublication {
            return Ok(());
        }
        self.publish_exact_receipt(binding, receipt)?;
        if seal_fault == FakeTerminalSealFault::LoseAcknowledgement {
            return Err(AdapterError::new(
                "terminal receipt acknowledgement was lost after publication",
            ));
        }
        Ok(())
    }

    pub(super) fn recover_receipts(
        &mut self,
        binding: &RunBindingReceipt,
    ) -> Result<Vec<RunTerminalReceipt>, AdapterError> {
        binding
            .validate()
            .map_err(|error| AdapterError::new(error.to_string()))?;
        self.recover_count = self.recover_count.wrapping_add(1);
        let receipts = self
            .recovery_receipts
            .clone()
            .unwrap_or_else(|| self.receipts(binding.receipt_digest()).to_vec());
        self.apply_readback_fault(receipts)
    }

    fn apply_readback_fault(
        &mut self,
        receipts: Vec<RunTerminalReceipt>,
    ) -> Result<Vec<RunTerminalReceipt>, AdapterError> {
        if self.readback_fault == FakeTerminalReadbackFault::ReturnError {
            self.readback_fault = FakeTerminalReadbackFault::None;
            return Err(AdapterError::new(
                "the reference terminal receipt read failed transiently",
            ));
        }
        if receipts.is_empty() {
            return Ok(receipts);
        }
        match std::mem::take(&mut self.readback_fault) {
            FakeTerminalReadbackFault::None => Ok(receipts),
            FakeTerminalReadbackFault::ChangeReceipt => Ok(vec![changed_receipt(&receipts[0])?]),
            FakeTerminalReadbackFault::TwoClasses => {
                let mut changed = receipts;
                changed.push(opposite_class_receipt(&changed[0])?);
                Ok(changed)
            }
            FakeTerminalReadbackFault::ReturnError => Err(AdapterError::new(
                "the reference terminal receipt read failed transiently",
            )),
        }
    }

    fn require_exact_binding(&self, binding: &RunBindingReceipt) -> Result<(), AdapterError> {
        binding
            .validate()
            .map_err(|error| AdapterError::new(error.to_string()))?;
        let saved = self
            .bound_plans
            .get(&binding.receipt_digest())
            .ok_or_else(|| AdapterError::new("the terminal plan binding does not exist"))?;
        if saved.binding != *binding {
            return Err(AdapterError::new(
                "the terminal operation has a foreign run binding",
            ));
        }
        Ok(())
    }

    fn publish_exact_receipt(
        &mut self,
        binding: &RunBindingReceipt,
        receipt: &RunTerminalReceipt,
    ) -> Result<(), AdapterError> {
        let saved = self.receipts.entry(binding.receipt_digest()).or_default();
        if saved.is_empty() {
            saved.push(receipt.clone());
            return Ok(());
        }
        if saved.len() == 1 && saved.first() == Some(receipt) {
            return Ok(());
        }
        Err(AdapterError::new(
            "the terminal receipt store contains a conflicting receipt",
        ))
    }
}

fn validate_binding_plan(
    binding: &RunBindingReceipt,
    plan: &RunTerminalPlan,
) -> Result<(), AdapterError> {
    binding
        .validate()
        .map_err(|error| AdapterError::new(error.to_string()))?;
    plan.validate()
        .map_err(|error| AdapterError::new(error.to_string()))?;
    if binding.terminal_plan_digest() != plan.plan_digest() {
        return Err(AdapterError::new(
            "the terminal plan differs from its exact run binding",
        ));
    }
    Ok(())
}

fn validate_exact_receipt(
    binding: &RunBindingReceipt,
    receipt: &RunTerminalReceipt,
) -> Result<(), AdapterError> {
    receipt
        .validate()
        .map_err(|error| AdapterError::new(error.to_string()))?;
    if receipt.binding() != binding {
        return Err(AdapterError::new(
            "the terminal receipt has a foreign run binding",
        ));
    }
    Ok(())
}

fn changed_receipt(receipt: &RunTerminalReceipt) -> Result<RunTerminalReceipt, AdapterError> {
    let first = stable_digest(83);
    let causal_digest = if receipt.causal_evidence_digest() == first {
        stable_digest(84)
    } else {
        first
    };
    RunTerminalReceipt::new(
        receipt.binding(),
        receipt.intent(),
        receipt.report(),
        receipt.class(),
        causal_digest,
    )
    .map_err(|error| AdapterError::new(error.to_string()))
}

fn opposite_class_receipt(
    receipt: &RunTerminalReceipt,
) -> Result<RunTerminalReceipt, AdapterError> {
    let class = RunTerminalClass::evidence_failure(receipt.intent(), receipt.report())
        .map_err(|error| AdapterError::new(error.to_string()))?;
    RunTerminalReceipt::new(
        receipt.binding(),
        receipt.intent(),
        receipt.report(),
        class,
        receipt.causal_evidence_digest(),
    )
    .map_err(|error| AdapterError::new(error.to_string()))
}

const fn operation_index(operation: RunTerminalOperation) -> usize {
    match operation {
        RunTerminalOperation::SimulatorStop => 0,
        RunTerminalOperation::ControlStop => 1,
        RunTerminalOperation::TraceStop => 2,
        RunTerminalOperation::ChildHealth => 3,
        RunTerminalOperation::TraceShutdown => 4,
        RunTerminalOperation::ChildTerminate => 5,
    }
}

const fn stable_digest(value: u8) -> Digest {
    Digest::from_bytes([value; 32])
}
