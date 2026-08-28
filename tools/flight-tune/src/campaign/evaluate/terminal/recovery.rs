use crate::journal::snapshot::{PreparedRunSnapshot, RunTerminalSnapshot};
use crate::{
    CampaignBackend, Journal, RunBindingReceipt, RunTerminalAdapter, RunTerminalBindingStatus,
    RunTerminalClass, RunTerminalDiagnostic, RunTerminalIntent, RunTerminalPlan,
    RunTerminalReceipt, RunTerminalRecoveryState, RunTerminalReport, RunTerminalScope,
    RunTerminalSemanticOutcome, SimulatorCapability, TuneError, VehicleBinding,
};

use super::{
    EvidenceBindingState, apply_binding_blocking, binding_for_run, execute_plan_blocking,
    plan_for_scope, seal_report_blocking,
};

pub(in crate::campaign::evaluate) fn recover_current_run_blocking<B, V>(
    journal: &mut Journal,
    run: &PreparedRunSnapshot,
    backend: &mut B,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
) -> Result<RunTerminalReceipt, TuneError>
where
    B: CampaignBackend,
    V: RunTerminalAdapter,
{
    match &run.terminal {
        RunTerminalSnapshot::Prepared => {
            recover_prepared_blocking(journal, run, backend, vehicle, capability)
        }
        RunTerminalSnapshot::Bound { plan, binding } => {
            recover_bound_blocking(journal, run, backend, vehicle, capability, binding, plan)
        }
        RunTerminalSnapshot::IntentPrepared {
            plan,
            binding,
            intent,
        } => report_and_seal_blocking(
            journal,
            run.run_index,
            backend,
            vehicle,
            capability,
            binding,
            plan,
            intent,
        ),
        RunTerminalSnapshot::ReportRecorded {
            binding,
            report,
            base_class,
            ..
        } => recover_saved_report_blocking(
            journal,
            run,
            vehicle,
            capability,
            binding,
            report,
            *base_class,
        ),
        RunTerminalSnapshot::EvidenceFailureRecorded {
            binding,
            report,
            class,
            ..
        } => recover_saved_report_blocking(
            journal, run, vehicle, capability, binding, report, *class,
        ),
        RunTerminalSnapshot::Committed { receipt } => Ok(receipt.as_ref().clone()),
    }
}

fn recover_prepared_blocking<B, V>(
    journal: &mut Journal,
    run: &PreparedRunSnapshot,
    backend: &mut B,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
) -> Result<RunTerminalReceipt, TuneError>
where
    B: CampaignBackend,
    V: RunTerminalAdapter,
{
    let plan = plan_for_scope(journal, vehicle, RunTerminalScope::RuntimeOnly)?;
    let binding = binding_for_run(journal, &run.context, &plan)?;
    let recovered = contain_without_intent_blocking(
        journal, run, backend, vehicle, capability, &binding, &plan,
    )?;
    journal.bind_run_terminal(
        run.context.trial_id(),
        run.run_index,
        plan.clone(),
        binding.clone(),
    )?;
    persist_contained_blocking(
        journal,
        run.run_index,
        vehicle,
        capability,
        &binding,
        recovered,
    )
}

#[allow(clippy::too_many_arguments)]
fn recover_saved_report_blocking<V>(
    journal: &mut Journal,
    run: &PreparedRunSnapshot,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    binding: &RunBindingReceipt,
    report: &crate::RunTerminalReport,
    class: RunTerminalClass,
) -> Result<RunTerminalReceipt, TuneError>
where
    V: RunTerminalAdapter,
{
    seal_report_blocking(
        journal,
        run.context.trial_id(),
        run.run_index,
        vehicle,
        capability,
        binding,
        report.plan(),
        report.intent(),
        report,
        class,
        EvidenceBindingState::RestoreIfAbsent,
    )
}

#[allow(clippy::too_many_arguments)]
fn recover_bound_blocking<B, V>(
    journal: &mut Journal,
    run: &PreparedRunSnapshot,
    backend: &mut B,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    binding: &RunBindingReceipt,
    plan: &RunTerminalPlan,
) -> Result<RunTerminalReceipt, TuneError>
where
    B: CampaignBackend,
    V: RunTerminalAdapter,
{
    let recovered =
        contain_without_intent_blocking(journal, run, backend, vehicle, capability, binding, plan)?;
    persist_contained_blocking(
        journal,
        run.run_index,
        vehicle,
        capability,
        binding,
        recovered,
    )
}

struct ContainedRun {
    intent: RunTerminalIntent,
    report: RunTerminalReport,
    class: RunTerminalClass,
}

#[allow(clippy::too_many_arguments)]
fn contain_without_intent_blocking<B, V>(
    journal: &Journal,
    run: &PreparedRunSnapshot,
    backend: &mut B,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    binding: &RunBindingReceipt,
    plan: &RunTerminalPlan,
) -> Result<ContainedRun, TuneError>
where
    B: CampaignBackend,
    V: RunTerminalAdapter,
{
    let intent = RunTerminalIntent::new(
        &run.context,
        run.run_intent_digest,
        RunTerminalSemanticOutcome::Recovery,
    )?;
    let report = execute_recovery_report_blocking(
        journal, backend, vehicle, capability, binding, plan, &intent,
    )?;
    let class = RunTerminalClass::classify(&intent, &report)?;
    Ok(ContainedRun {
        intent,
        report,
        class,
    })
}

#[allow(clippy::too_many_arguments)]
fn report_and_seal_blocking<B, V>(
    journal: &mut Journal,
    run_index: u64,
    backend: &mut B,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    binding: &RunBindingReceipt,
    plan: &RunTerminalPlan,
    intent: &RunTerminalIntent,
) -> Result<RunTerminalReceipt, TuneError>
where
    B: CampaignBackend,
    V: RunTerminalAdapter,
{
    let report = execute_recovery_report_blocking(
        journal, backend, vehicle, capability, binding, plan, intent,
    )?;
    let class = RunTerminalClass::classify(intent, &report)?;
    seal_report_blocking(
        journal,
        intent.context().trial_id(),
        run_index,
        vehicle,
        capability,
        binding,
        plan,
        intent,
        &report,
        class,
        EvidenceBindingState::Active,
    )
}

#[allow(clippy::too_many_arguments)]
fn persist_contained_blocking<V>(
    journal: &mut Journal,
    run_index: u64,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    binding: &RunBindingReceipt,
    recovered: ContainedRun,
) -> Result<RunTerminalReceipt, TuneError>
where
    V: RunTerminalAdapter,
{
    let trial_id = recovered.intent.context().trial_id();
    journal.prepare_run_terminal_intent(trial_id, run_index, recovered.intent.clone())?;
    seal_report_blocking(
        journal,
        trial_id,
        run_index,
        vehicle,
        capability,
        binding,
        recovered.report.plan(),
        &recovered.intent,
        &recovered.report,
        recovered.class,
        EvidenceBindingState::Active,
    )
}

#[allow(clippy::too_many_arguments)]
fn execute_recovery_report_blocking<B, V>(
    journal: &Journal,
    backend: &mut B,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    binding: &RunBindingReceipt,
    plan: &RunTerminalPlan,
    intent: &RunTerminalIntent,
) -> Result<RunTerminalReport, TuneError>
where
    B: CampaignBackend,
    V: RunTerminalAdapter,
{
    let binding_status =
        apply_recovery_binding_blocking(journal, vehicle, capability, binding, plan)?;
    execute_plan_blocking(
        journal,
        backend,
        vehicle,
        capability,
        binding,
        plan,
        intent,
        RunTerminalRecoveryState::Resumed,
        binding_status,
    )
}

fn apply_recovery_binding_blocking<V>(
    journal: &Journal,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    binding: &RunBindingReceipt,
    plan: &RunTerminalPlan,
) -> Result<RunTerminalBindingStatus, TuneError>
where
    V: RunTerminalAdapter,
{
    match apply_binding_blocking(journal, vehicle, capability, binding, plan) {
        Ok(()) => Ok(RunTerminalBindingStatus::Succeeded),
        Err(error) => {
            journal.ensure_usable()?;
            let diagnostic =
                RunTerminalDiagnostic::new(&format!("terminal plan binding failed: {error}"))?;
            Ok(RunTerminalBindingStatus::Failed { diagnostic })
        }
    }
}
