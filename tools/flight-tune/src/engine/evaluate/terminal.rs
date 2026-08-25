mod evidence;
mod recovery;

use crate::{
    AdapterError, Digest, Journal, RunBindingReceipt, RunExecutionContext, RunTerminalAdapter,
    RunTerminalBindingStatus, RunTerminalClass, RunTerminalDiagnostic, RunTerminalIntent,
    RunTerminalOperation, RunTerminalOperationOutcome, RunTerminalPlan, RunTerminalReceipt,
    RunTerminalRecoveryState, RunTerminalReport, RunTerminalScope, RunTerminalSemanticOutcome,
    SearchStage, SimulatorBackend, SimulatorCapability, TuneError, VehicleBinding,
};

pub(super) use evidence::{EvidenceBindingState, seal_report_blocking};
pub(super) use recovery::recover_current_run_blocking;

use super::contract::{RunContext, RunTerminal, run_record};

pub(super) struct LiveTerminalBinding {
    pub(super) plan: RunTerminalPlan,
    pub(super) binding: RunBindingReceipt,
    binding_status: RunTerminalBindingStatus,
    pub(super) binding_error: Option<TuneError>,
}

pub(super) struct CommittedTerminal {
    pub(super) receipt: RunTerminalReceipt,
    pub(super) primary_error: Option<TuneError>,
}

pub(super) fn prepare_live_terminal_blocking<V>(
    journal: &mut Journal,
    context: &RunContext<'_>,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
) -> Result<LiveTerminalBinding, TuneError>
where
    V: RunTerminalAdapter,
{
    let plan = plan_for_scope(journal, vehicle, RunTerminalScope::Active)?;
    let binding = binding_for_run(journal, &context.execution, &plan)?;
    journal.bind_run_terminal(
        context.execution.trial_id(),
        context.run_index,
        plan.clone(),
        binding.clone(),
    )?;
    let (binding_status, binding_error) = binding_result(apply_binding_blocking(
        journal, vehicle, capability, &binding, &plan,
    ))?;
    journal.ensure_usable()?;
    Ok(LiveTerminalBinding {
        plan,
        binding,
        binding_status,
        binding_error,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn finish_live_terminal_blocking<B, V>(
    journal: &mut Journal,
    stage: &SearchStage,
    context: &RunContext<'_>,
    terminal: RunTerminal,
    prepared: &LiveTerminalBinding,
    backend: &mut B,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
) -> Result<CommittedTerminal, TuneError>
where
    B: SimulatorBackend,
    V: RunTerminalAdapter,
{
    let (outcome, mut primary_error) = semantic_outcome(stage, context, terminal)?;
    let intent = RunTerminalIntent::new(&context.execution, context.run_intent_digest, outcome)?;
    let receipt = match persist_live_terminal_blocking(
        journal, context, &intent, prepared, backend, vehicle, capability,
    ) {
        Ok(receipt) => receipt,
        Err(error) => return Err(prefer_primary(journal, primary_error.take(), error)),
    };
    let primary_error = primary_error.or_else(|| {
        (!receipt.is_completed()).then_some(TuneError::InvalidState {
            operation: "evaluate candidate",
            detail: "the terminal receipt quarantined the run".to_owned(),
        })
    });
    Ok(CommittedTerminal {
        receipt,
        primary_error,
    })
}

#[allow(clippy::too_many_arguments)]
fn persist_live_terminal_blocking<B, V>(
    journal: &mut Journal,
    context: &RunContext<'_>,
    intent: &RunTerminalIntent,
    prepared: &LiveTerminalBinding,
    backend: &mut B,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
) -> Result<RunTerminalReceipt, TuneError>
where
    B: SimulatorBackend,
    V: RunTerminalAdapter,
{
    let trial_id = context.execution.trial_id();
    let run_index = context.run_index;
    journal.prepare_run_terminal_intent(trial_id, run_index, intent.clone())?;
    let report = execute_plan_blocking(
        journal,
        backend,
        vehicle,
        capability,
        &prepared.binding,
        &prepared.plan,
        intent,
        RunTerminalRecoveryState::Live,
        prepared.binding_status.clone(),
    )?;
    let class = RunTerminalClass::classify(intent, &report)?;
    seal_report_blocking(
        journal,
        trial_id,
        run_index,
        vehicle,
        capability,
        &prepared.binding,
        &prepared.plan,
        intent,
        &report,
        class,
        EvidenceBindingState::Active,
    )
}

fn prefer_primary(
    journal: &Journal,
    primary: Option<TuneError>,
    terminal_error: TuneError,
) -> TuneError {
    match primary {
        None => terminal_error,
        Some(primary) if journal.ensure_usable().is_err() => primary,
        Some(primary) => TuneError::OperationAndTerminalFailed {
            operation: "evaluate candidate",
            primary: Box::new(primary),
            terminal: Box::new(terminal_error),
        },
    }
}

fn semantic_outcome(
    stage: &SearchStage,
    context: &RunContext<'_>,
    terminal: RunTerminal,
) -> Result<(RunTerminalSemanticOutcome, Option<TuneError>), TuneError> {
    match terminal {
        RunTerminal::Passed { values } => Ok((
            RunTerminalSemanticOutcome::ScenarioComplete {
                candidate_digest: context.execution.candidate_digest(),
                scenario_digest: context.execution.scenario_digest(),
                run: run_record(stage, context, values),
            },
            None,
        )),
        RunTerminal::HardGate { failure } => Ok((
            RunTerminalSemanticOutcome::HardGateAbort {
                candidate_digest: context.execution.candidate_digest(),
                scenario_digest: context.execution.scenario_digest(),
                failure,
            },
            None,
        )),
        RunTerminal::Failed { error } => {
            let diagnostic = RunTerminalDiagnostic::new(&error.to_string())?;
            Ok((
                RunTerminalSemanticOutcome::ExecutionError { diagnostic },
                Some(error),
            ))
        }
    }
}

pub(super) fn plan_for_scope<V>(
    journal: &Journal,
    vehicle: &VehicleBinding<V>,
    scope: RunTerminalScope,
) -> Result<RunTerminalPlan, TuneError>
where
    V: RunTerminalAdapter,
{
    journal.ensure_usable()?;
    let plan = vehicle
        .terminal_plan_for_scope(scope)
        .map_err(|source| terminal_error("create terminal plan", source))?;
    journal.ensure_usable()?;
    Ok(plan)
}

pub(super) fn binding_for_run(
    journal: &Journal,
    context: &RunExecutionContext,
    plan: &RunTerminalPlan,
) -> Result<RunBindingReceipt, TuneError> {
    journal.ensure_usable()?;
    RunBindingReceipt::new(context, plan, journal.session().runtimes.vehicle.clone())
}

pub(super) fn apply_binding_blocking<V>(
    journal: &Journal,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    binding: &RunBindingReceipt,
    plan: &RunTerminalPlan,
) -> Result<(), TuneError>
where
    V: RunTerminalAdapter,
{
    journal.ensure_usable()?;
    vehicle
        .bind_terminal_plan_blocking(capability, binding, plan)
        .map_err(|source| terminal_error("bind terminal plan", source))?;
    journal.ensure_usable()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn execute_plan_blocking<B, V>(
    journal: &Journal,
    backend: &mut B,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    binding: &RunBindingReceipt,
    plan: &RunTerminalPlan,
    intent: &RunTerminalIntent,
    recovery_state: RunTerminalRecoveryState,
    binding_status: RunTerminalBindingStatus,
) -> Result<RunTerminalReport, TuneError>
where
    B: SimulatorBackend,
    V: RunTerminalAdapter,
{
    plan.validate()?;
    let mut outcomes = Vec::with_capacity(plan.requirements().len());
    for requirement in plan.requirements().iter().copied() {
        let operation = requirement.operation();
        let outcome = if requirement.is_required() {
            execute_required_blocking(
                journal, backend, vehicle, capability, binding, plan, operation,
            )?
        } else {
            RunTerminalOperationOutcome::not_required(operation)
        };
        outcomes.push(outcome);
    }
    journal.ensure_usable()?;
    RunTerminalReport::new_with_binding_status(
        plan,
        intent,
        recovery_state,
        binding_status,
        outcomes,
    )
}

fn binding_result(
    result: Result<(), TuneError>,
) -> Result<(RunTerminalBindingStatus, Option<TuneError>), TuneError> {
    match result {
        Ok(()) => Ok((RunTerminalBindingStatus::Succeeded, None)),
        Err(error) => {
            let diagnostic = RunTerminalDiagnostic::new(&error.to_string())?;
            Ok((RunTerminalBindingStatus::Failed { diagnostic }, Some(error)))
        }
    }
}

pub(super) fn causal_evidence_digest_blocking<V>(
    journal: &Journal,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    binding: &RunBindingReceipt,
    plan: &RunTerminalPlan,
) -> Result<Digest, TuneError>
where
    V: RunTerminalAdapter,
{
    journal.ensure_usable()?;
    let digest = vehicle
        .terminal_causal_evidence_digest_blocking(capability, binding, plan)
        .map_err(|source| terminal_error("read causal evidence", source))?;
    journal.ensure_usable()?;
    Ok(digest)
}

#[allow(clippy::too_many_arguments)]
fn execute_required_blocking<B, V>(
    journal: &Journal,
    backend: &mut B,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    binding: &RunBindingReceipt,
    plan: &RunTerminalPlan,
    operation: RunTerminalOperation,
) -> Result<RunTerminalOperationOutcome, TuneError>
where
    B: SimulatorBackend,
    V: RunTerminalAdapter,
{
    journal.ensure_usable()?;
    let result = run_operation_blocking(backend, vehicle, capability, binding, plan, operation);
    match result {
        Ok(proof) => RunTerminalOperationOutcome::succeeded(operation, proof),
        Err(source) => RunTerminalOperationOutcome::failed(
            operation,
            RunTerminalDiagnostic::new(&format!("{}: {source}", operation_name(operation)))?,
        ),
    }
}

fn run_operation_blocking<B, V>(
    backend: &mut B,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    binding: &RunBindingReceipt,
    plan: &RunTerminalPlan,
    operation: RunTerminalOperation,
) -> Result<Option<Digest>, AdapterError>
where
    B: SimulatorBackend,
    V: RunTerminalAdapter,
{
    match operation {
        RunTerminalOperation::SimulatorStop => backend.stop_blocking().map(|()| None),
        RunTerminalOperation::ControlStop => {
            vehicle.terminal_control_stop_blocking(capability, binding, plan)
        }
        RunTerminalOperation::TraceStop => {
            vehicle.terminal_trace_stop_blocking(capability, binding, plan)
        }
        RunTerminalOperation::ChildHealth => {
            vehicle.terminal_child_health_blocking(capability, binding, plan)
        }
        RunTerminalOperation::TraceShutdown => {
            vehicle.terminal_trace_shutdown_blocking(capability, binding, plan)
        }
        RunTerminalOperation::ChildTerminate => vehicle
            .terminal_child_terminate_blocking(capability, binding, plan)
            .map(Some),
    }
}

const fn operation_name(operation: RunTerminalOperation) -> &'static str {
    match operation {
        RunTerminalOperation::SimulatorStop => "simulator stop",
        RunTerminalOperation::ControlStop => "control stop",
        RunTerminalOperation::TraceStop => "trace stop",
        RunTerminalOperation::ChildHealth => "child health",
        RunTerminalOperation::TraceShutdown => "trace shutdown",
        RunTerminalOperation::ChildTerminate => "child terminate",
    }
}

fn terminal_error(operation: &'static str, source: AdapterError) -> TuneError {
    TuneError::Adapter {
        adapter: "bound terminal adapter".to_owned(),
        operation,
        source,
    }
}
