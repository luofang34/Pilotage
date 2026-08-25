use pilotage_trial::Digest;

use super::{AdapterError, SimulatorCapability, VehicleBinding};
use crate::terminal::{
    RUN_TERMINAL_OPERATION_ORDER, RunBindingReceipt, RunTerminalOperation, RunTerminalPlan,
    RunTerminalReceipt, RunTerminalRequirement, RunTerminalScope,
};

/// The external components that one terminal adapter controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunTerminalCapabilities {
    control: bool,
    trace: bool,
    supervised_child: bool,
}

impl RunTerminalCapabilities {
    /// Creates an immutable terminal capability set.
    #[must_use]
    pub const fn new(control: bool, trace: bool, supervised_child: bool) -> Self {
        Self {
            control,
            trace,
            supervised_child,
        }
    }

    /// Reports whether the adapter controls a vehicle control path.
    #[must_use]
    pub const fn has_control(self) -> bool {
        self.control
    }

    /// Reports whether the adapter controls a trace path.
    #[must_use]
    pub const fn has_trace(self) -> bool {
        self.trace
    }

    /// Reports whether the adapter controls a supervised child group.
    #[must_use]
    pub const fn has_supervised_child(self) -> bool {
        self.supervised_child
    }

    pub(crate) fn plan_for_scope(
        self,
        scope: RunTerminalScope,
    ) -> Result<RunTerminalPlan, AdapterError> {
        let requirements = RUN_TERMINAL_OPERATION_ORDER.map(|operation| {
            terminal_requirement(operation, self.operation_is_required(scope, operation))
        });
        RunTerminalPlan::from_requirements(scope, requirements)
            .map_err(|error| AdapterError::new(error.to_string()))
    }

    const fn operation_is_required(
        self,
        scope: RunTerminalScope,
        operation: RunTerminalOperation,
    ) -> bool {
        if matches!(scope, RunTerminalScope::NeverStarted) {
            return false;
        }
        match operation {
            RunTerminalOperation::SimulatorStop => matches!(scope, RunTerminalScope::Active),
            RunTerminalOperation::ControlStop => self.control,
            RunTerminalOperation::TraceStop | RunTerminalOperation::TraceShutdown => self.trace,
            RunTerminalOperation::ChildHealth | RunTerminalOperation::ChildTerminate => {
                self.supervised_child
            }
        }
    }
}

const fn terminal_requirement(
    operation: RunTerminalOperation,
    required: bool,
) -> RunTerminalRequirement {
    if required {
        RunTerminalRequirement::Required { operation }
    } else {
        RunTerminalRequirement::NotRequired { operation }
    }
}

/// An independent adapter for idempotent terminal operations and evidence.
pub trait RunTerminalAdapter {
    /// Returns the fixed external component capabilities.
    fn terminal_capabilities(&self) -> RunTerminalCapabilities;

    /// Idempotently binds one exact immutable terminal plan.
    fn bind_terminal_plan_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _binding: &RunBindingReceipt,
        _plan: &RunTerminalPlan,
    ) -> Result<(), AdapterError> {
        Err(unsupported_terminal_operation("bind terminal plan"))
    }

    /// Idempotently stops the vehicle control path.
    fn control_stop_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _binding: &RunBindingReceipt,
    ) -> Result<Option<Digest>, AdapterError> {
        Err(unsupported_terminal_operation("stop control path"))
    }

    /// Idempotently stops trace collection.
    fn trace_stop_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _binding: &RunBindingReceipt,
    ) -> Result<Option<Digest>, AdapterError> {
        Err(unsupported_terminal_operation("stop trace collection"))
    }

    /// Idempotently checks the supervised child state.
    fn child_health_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _binding: &RunBindingReceipt,
    ) -> Result<Option<Digest>, AdapterError> {
        Err(unsupported_terminal_operation("check child health"))
    }

    /// Idempotently joins the trace path.
    fn trace_shutdown_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _binding: &RunBindingReceipt,
    ) -> Result<Option<Digest>, AdapterError> {
        Err(unsupported_terminal_operation("join trace path"))
    }

    /// Idempotently terminates and reaps the supervised child group.
    fn child_terminate_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _binding: &RunBindingReceipt,
    ) -> Result<Digest, AdapterError> {
        Err(unsupported_terminal_operation("terminate child group"))
    }

    /// Reads the exact causal evidence identity.
    fn causal_evidence_digest_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _binding: &RunBindingReceipt,
    ) -> Result<Digest, AdapterError> {
        Err(unsupported_terminal_operation("read causal evidence"))
    }

    /// Seals one exact core-supplied terminal receipt.
    fn seal_terminal_receipt_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _binding: &RunBindingReceipt,
        _receipt: &RunTerminalReceipt,
    ) -> Result<(), AdapterError> {
        Err(unsupported_terminal_operation("seal terminal receipt"))
    }

    /// Recovers all terminal receipts for one exact run binding.
    fn recover_terminal_receipts_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _binding: &RunBindingReceipt,
    ) -> Result<Vec<RunTerminalReceipt>, AdapterError> {
        Err(unsupported_terminal_operation("recover terminal receipts"))
    }
}

#[allow(dead_code)]
impl<A: RunTerminalAdapter> VehicleBinding<A> {
    pub(crate) fn terminal_plan_for_scope(
        &self,
        scope: RunTerminalScope,
    ) -> Result<RunTerminalPlan, AdapterError> {
        self.adapter.terminal_capabilities().plan_for_scope(scope)
    }

    pub(crate) fn bind_terminal_plan_blocking(
        &mut self,
        capability: &SimulatorCapability,
        binding: &RunBindingReceipt,
        plan: &RunTerminalPlan,
    ) -> Result<(), AdapterError> {
        self.validate_terminal_call(capability, binding, plan)?;
        self.adapter
            .bind_terminal_plan_blocking(capability, binding, plan)
    }

    pub(crate) fn terminal_control_stop_blocking(
        &mut self,
        capability: &SimulatorCapability,
        binding: &RunBindingReceipt,
        plan: &RunTerminalPlan,
    ) -> Result<Option<Digest>, AdapterError> {
        self.validate_terminal_call(capability, binding, plan)?;
        let proof = self.adapter.control_stop_blocking(capability, binding)?;
        validate_optional_proof(proof)
    }

    pub(crate) fn terminal_trace_stop_blocking(
        &mut self,
        capability: &SimulatorCapability,
        binding: &RunBindingReceipt,
        plan: &RunTerminalPlan,
    ) -> Result<Option<Digest>, AdapterError> {
        self.validate_terminal_call(capability, binding, plan)?;
        let proof = self.adapter.trace_stop_blocking(capability, binding)?;
        validate_optional_proof(proof)
    }

    pub(crate) fn terminal_child_health_blocking(
        &mut self,
        capability: &SimulatorCapability,
        binding: &RunBindingReceipt,
        plan: &RunTerminalPlan,
    ) -> Result<Option<Digest>, AdapterError> {
        self.validate_terminal_call(capability, binding, plan)?;
        let proof = self.adapter.child_health_blocking(capability, binding)?;
        validate_optional_proof(proof)
    }

    pub(crate) fn terminal_trace_shutdown_blocking(
        &mut self,
        capability: &SimulatorCapability,
        binding: &RunBindingReceipt,
        plan: &RunTerminalPlan,
    ) -> Result<Option<Digest>, AdapterError> {
        self.validate_terminal_call(capability, binding, plan)?;
        let proof = self.adapter.trace_shutdown_blocking(capability, binding)?;
        validate_optional_proof(proof)
    }

    pub(crate) fn terminal_child_terminate_blocking(
        &mut self,
        capability: &SimulatorCapability,
        binding: &RunBindingReceipt,
        plan: &RunTerminalPlan,
    ) -> Result<Digest, AdapterError> {
        self.validate_terminal_call(capability, binding, plan)?;
        let proof = self.adapter.child_terminate_blocking(capability, binding)?;
        validate_required_proof(proof)
    }

    pub(crate) fn terminal_causal_evidence_digest_blocking(
        &mut self,
        capability: &SimulatorCapability,
        binding: &RunBindingReceipt,
        plan: &RunTerminalPlan,
    ) -> Result<Digest, AdapterError> {
        self.validate_terminal_call(capability, binding, plan)?;
        let digest = self
            .adapter
            .causal_evidence_digest_blocking(capability, binding)?;
        validate_required_proof(digest)
    }

    pub(crate) fn seal_terminal_receipt_blocking(
        &mut self,
        capability: &SimulatorCapability,
        binding: &RunBindingReceipt,
        plan: &RunTerminalPlan,
        receipt: &RunTerminalReceipt,
    ) -> Result<(), AdapterError> {
        self.validate_terminal_call(capability, binding, plan)?;
        validate_exact_receipt(receipt, binding)?;
        self.adapter
            .seal_terminal_receipt_blocking(capability, binding, receipt)
    }

    pub(crate) fn recover_terminal_receipts_blocking(
        &mut self,
        capability: &SimulatorCapability,
        binding: &RunBindingReceipt,
        plan: &RunTerminalPlan,
    ) -> Result<Vec<RunTerminalReceipt>, AdapterError> {
        self.validate_terminal_call(capability, binding, plan)?;
        self.adapter
            .recover_terminal_receipts_blocking(capability, binding)
    }

    fn validate_terminal_call(
        &self,
        capability: &SimulatorCapability,
        binding: &RunBindingReceipt,
        plan: &RunTerminalPlan,
    ) -> Result<(), AdapterError> {
        binding
            .validate()
            .map_err(|error| AdapterError::new(error.to_string()))?;
        plan.validate()
            .map_err(|error| AdapterError::new(error.to_string()))?;
        if self.receipt.session_digest != capability.session_digest()
            || binding.context().tuning_session_digest() != capability.session_digest()
            || binding.adapter().digest != self.receipt.vehicle_digest
            || binding.terminal_plan_digest() != plan.plan_digest()
        {
            return Err(AdapterError::new(
                "the terminal call differs from its simulator or vehicle binding",
            ));
        }
        Ok(())
    }
}

fn validate_optional_proof(proof: Option<Digest>) -> Result<Option<Digest>, AdapterError> {
    if proof.is_some_and(Digest::is_zero) {
        return Err(AdapterError::new(
            "a terminal operation returned a zero durable proof",
        ));
    }
    Ok(proof)
}

fn validate_required_proof(proof: Digest) -> Result<Digest, AdapterError> {
    if proof.is_zero() {
        return Err(AdapterError::new(
            "a terminal operation returned a zero durable proof",
        ));
    }
    Ok(proof)
}

fn validate_exact_receipt(
    receipt: &RunTerminalReceipt,
    binding: &RunBindingReceipt,
) -> Result<(), AdapterError> {
    receipt
        .validate()
        .map_err(|error| AdapterError::new(error.to_string()))?;
    if receipt.binding() != binding {
        return Err(AdapterError::new(
            "a terminal receipt differs from its exact run binding",
        ));
    }
    Ok(())
}

fn unsupported_terminal_operation(operation: &'static str) -> AdapterError {
    AdapterError::new(format!("terminal adapter does not support {operation}"))
}

#[cfg(test)]
mod tests;
