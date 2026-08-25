use flight_tune::{
    AdapterError, Digest, RunBindingReceipt, RunTerminalAdapter, RunTerminalCapabilities,
    RunTerminalOperation, RunTerminalPlan, RunTerminalReceipt, SimulatorCapability,
};

use super::FakeVehicle;
use super::terminal_head_poison::{TerminalExternalAction, poison_terminal_head};

impl RunTerminalAdapter for FakeVehicle {
    fn terminal_capabilities(&self) -> RunTerminalCapabilities {
        let (capabilities, head_poison) = self.state.0.borrow_mut().terminal.read_capabilities();
        poison_terminal_head(head_poison);
        capabilities
    }

    fn bind_terminal_plan_blocking(
        &mut self,
        capability: &SimulatorCapability,
        binding: &RunBindingReceipt,
        plan: &RunTerminalPlan,
    ) -> Result<(), AdapterError> {
        validate_capability(capability, binding)?;
        let (result, head_poison) = {
            let mut state = self.state.0.borrow_mut();
            let result = state.terminal.bind_plan(binding, plan);
            state.lifecycle.push("bind_terminal_plan".to_owned());
            let head_poison = state
                .terminal
                .take_head_poison(TerminalExternalAction::Bind);
            (result, head_poison)
        };
        poison_terminal_head(head_poison);
        result
    }

    fn control_stop_blocking(
        &mut self,
        capability: &SimulatorCapability,
        binding: &RunBindingReceipt,
    ) -> Result<Option<Digest>, AdapterError> {
        run_operation(self, capability, binding, RunTerminalOperation::ControlStop)
    }

    fn trace_stop_blocking(
        &mut self,
        capability: &SimulatorCapability,
        binding: &RunBindingReceipt,
    ) -> Result<Option<Digest>, AdapterError> {
        run_operation(self, capability, binding, RunTerminalOperation::TraceStop)
    }

    fn child_health_blocking(
        &mut self,
        capability: &SimulatorCapability,
        binding: &RunBindingReceipt,
    ) -> Result<Option<Digest>, AdapterError> {
        run_operation(self, capability, binding, RunTerminalOperation::ChildHealth)
    }

    fn trace_shutdown_blocking(
        &mut self,
        capability: &SimulatorCapability,
        binding: &RunBindingReceipt,
    ) -> Result<Option<Digest>, AdapterError> {
        run_operation(
            self,
            capability,
            binding,
            RunTerminalOperation::TraceShutdown,
        )
    }

    fn child_terminate_blocking(
        &mut self,
        capability: &SimulatorCapability,
        binding: &RunBindingReceipt,
    ) -> Result<Digest, AdapterError> {
        run_operation(
            self,
            capability,
            binding,
            RunTerminalOperation::ChildTerminate,
        )?
        .ok_or_else(|| AdapterError::new("child termination has no durable proof"))
    }

    fn causal_evidence_digest_blocking(
        &mut self,
        capability: &SimulatorCapability,
        binding: &RunBindingReceipt,
    ) -> Result<Digest, AdapterError> {
        validate_capability(capability, binding)?;
        let (result, head_poison) = {
            let mut state = self.state.0.borrow_mut();
            let result = state.terminal.read_causal_evidence(binding);
            state.lifecycle.push("read_causal_evidence".to_owned());
            let head_poison = state
                .terminal
                .take_head_poison(TerminalExternalAction::CausalRead);
            (result, head_poison)
        };
        poison_terminal_head(head_poison);
        result
    }

    fn seal_terminal_receipt_blocking(
        &mut self,
        capability: &SimulatorCapability,
        binding: &RunBindingReceipt,
        receipt: &RunTerminalReceipt,
    ) -> Result<(), AdapterError> {
        validate_capability(capability, binding)?;
        let (result, head_poison) = {
            let mut state = self.state.0.borrow_mut();
            let result = state.terminal.seal_receipt(binding, receipt);
            state.lifecycle.push("seal_terminal_receipt".to_owned());
            let head_poison = state
                .terminal
                .take_head_poison(TerminalExternalAction::ReceiptSeal);
            (result, head_poison)
        };
        poison_terminal_head(head_poison);
        result
    }

    fn recover_terminal_receipts_blocking(
        &mut self,
        capability: &SimulatorCapability,
        binding: &RunBindingReceipt,
    ) -> Result<Vec<RunTerminalReceipt>, AdapterError> {
        validate_capability(capability, binding)?;
        let (result, head_poison) = {
            let mut state = self.state.0.borrow_mut();
            let result = state.terminal.recover_receipts(binding);
            state.lifecycle.push("recover_terminal_receipts".to_owned());
            let head_poison = state
                .terminal
                .take_head_poison(TerminalExternalAction::ReceiptRecover);
            (result, head_poison)
        };
        poison_terminal_head(head_poison);
        result
    }
}

fn run_operation(
    vehicle: &mut FakeVehicle,
    capability: &SimulatorCapability,
    binding: &RunBindingReceipt,
    operation: RunTerminalOperation,
) -> Result<Option<Digest>, AdapterError> {
    validate_capability(capability, binding)?;
    let (result, head_poison) = {
        let mut state = vehicle.state.0.borrow_mut();
        let result = state.terminal.run_operation(binding, operation);
        state.lifecycle.push(operation_label(operation).to_owned());
        let head_poison = state.terminal.take_head_poison(external_action(operation));
        (result, head_poison)
    };
    poison_terminal_head(head_poison);
    result
}

const fn external_action(operation: RunTerminalOperation) -> TerminalExternalAction {
    match operation {
        RunTerminalOperation::SimulatorStop => TerminalExternalAction::SimulatorStop,
        RunTerminalOperation::ControlStop => TerminalExternalAction::ControlStop,
        RunTerminalOperation::TraceStop => TerminalExternalAction::TraceStop,
        RunTerminalOperation::ChildHealth => TerminalExternalAction::ChildHealth,
        RunTerminalOperation::TraceShutdown => TerminalExternalAction::TraceShutdown,
        RunTerminalOperation::ChildTerminate => TerminalExternalAction::ChildTerminate,
    }
}

fn validate_capability(
    capability: &SimulatorCapability,
    binding: &RunBindingReceipt,
) -> Result<(), AdapterError> {
    binding
        .validate()
        .map_err(|error| AdapterError::new(error.to_string()))?;
    if binding.context().tuning_session_digest() != capability.session_digest() {
        return Err(AdapterError::new(
            "the reference terminal adapter received a foreign simulator session",
        ));
    }
    Ok(())
}

const fn operation_label(operation: RunTerminalOperation) -> &'static str {
    match operation {
        RunTerminalOperation::SimulatorStop => "stop",
        RunTerminalOperation::ControlStop => "terminal_control_stop",
        RunTerminalOperation::TraceStop => "terminal_trace_stop",
        RunTerminalOperation::ChildHealth => "terminal_child_health",
        RunTerminalOperation::TraceShutdown => "terminal_trace_shutdown",
        RunTerminalOperation::ChildTerminate => "terminal_child_terminate",
    }
}

#[cfg(test)]
#[path = "terminal/tests.rs"]
mod tests;
