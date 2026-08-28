use crate::journal::snapshot::RunTerminalSnapshot;
use crate::{
    Journal, RunBindingReceipt, RunTerminalAdapter, RunTerminalClass, RunTerminalIntent,
    RunTerminalPlan, RunTerminalReceipt, RunTerminalReport, SimulatorCapability, TuneError,
    VehicleBinding,
};

use super::terminal_error;

#[derive(Clone, Copy)]
pub(in crate::campaign::evaluate) enum EvidenceBindingState {
    Active,
    RestoreIfAbsent,
}

#[allow(clippy::too_many_arguments)]
pub(in crate::campaign::evaluate) fn seal_report_blocking<V>(
    journal: &mut Journal,
    trial_id: u64,
    run_index: u64,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    binding: &RunBindingReceipt,
    plan: &RunTerminalPlan,
    intent: &RunTerminalIntent,
    report: &RunTerminalReport,
    class: RunTerminalClass,
    binding_state: EvidenceBindingState,
) -> Result<RunTerminalReceipt, TuneError>
where
    V: RunTerminalAdapter,
{
    let desired = resolve_authority_blocking(
        journal, trial_id, run_index, vehicle, capability, binding, plan, intent, report, class,
    )?;
    if let Some(receipt) = recover_authoritative_receipt_blocking(
        journal, vehicle, capability, binding, plan, &desired,
    )? {
        return commit_receipt(journal, trial_id, run_index, receipt);
    }
    if matches!(binding_state, EvidenceBindingState::RestoreIfAbsent) {
        super::apply_binding_blocking(journal, vehicle, capability, binding, plan)?;
    }

    let publication =
        publish_and_read_blocking(journal, vehicle, capability, binding, plan, &desired)?;
    finish_publication_blocking(
        journal,
        trial_id,
        run_index,
        vehicle,
        capability,
        binding,
        plan,
        &desired,
        publication,
    )
}

#[allow(clippy::too_many_arguments)]
fn resolve_authority_blocking<V>(
    journal: &mut Journal,
    trial_id: u64,
    run_index: u64,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    binding: &RunBindingReceipt,
    plan: &RunTerminalPlan,
    intent: &RunTerminalIntent,
    report: &RunTerminalReport,
    class: RunTerminalClass,
) -> Result<RunTerminalReceipt, TuneError>
where
    V: RunTerminalAdapter,
{
    let terminal = authority_terminal(journal, trial_id, run_index)?;
    match terminal {
        RunTerminalSnapshot::IntentPrepared {
            plan: saved_plan,
            binding: saved_binding,
            intent: saved_intent,
        } => {
            require_authority_inputs(
                &saved_binding,
                &saved_plan,
                &saved_intent,
                binding,
                plan,
                intent,
            )?;
            record_authority_blocking(
                journal, trial_id, run_index, vehicle, capability, binding, plan, intent, report,
                class,
            )
        }
        RunTerminalSnapshot::ReportRecorded {
            binding: saved_binding,
            report: saved_report,
            base_class,
            expected_receipt,
        } => {
            require_saved_report(
                &saved_binding,
                &saved_report,
                base_class,
                binding,
                report,
                class,
            )?;
            Ok(*expected_receipt)
        }
        RunTerminalSnapshot::EvidenceFailureRecorded {
            binding: saved_binding,
            report: saved_report,
            expected_receipt,
            class: saved_class,
        } => {
            require_saved_report(
                &saved_binding,
                &saved_report,
                saved_class,
                binding,
                report,
                class,
            )?;
            RunTerminalReceipt::new(
                binding,
                intent,
                report,
                class,
                expected_receipt.causal_evidence_digest(),
            )
        }
        _ => Err(missing_authority_state()),
    }
}

fn authority_terminal(
    journal: &Journal,
    trial_id: u64,
    run_index: u64,
) -> Result<RunTerminalSnapshot, TuneError> {
    journal
        .pending_attempt_snapshot()
        .and_then(|pending| {
            (pending.trial_id == trial_id)
                .then_some(pending)
                .and_then(|pending| pending.current_run().cloned())
        })
        .filter(|run| run.run_index == run_index)
        .map(|run| run.terminal)
        .ok_or_else(missing_authority_state)
}

#[allow(clippy::too_many_arguments)]
fn record_authority_blocking<V>(
    journal: &mut Journal,
    trial_id: u64,
    run_index: u64,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    binding: &RunBindingReceipt,
    plan: &RunTerminalPlan,
    intent: &RunTerminalIntent,
    report: &RunTerminalReport,
    class: RunTerminalClass,
) -> Result<RunTerminalReceipt, TuneError>
where
    V: RunTerminalAdapter,
{
    let causal_digest =
        super::causal_evidence_digest_blocking(journal, vehicle, capability, binding, plan)?;
    let desired = RunTerminalReceipt::new(binding, intent, report, class, causal_digest)?;
    journal.record_run_terminal_report(
        trial_id,
        run_index,
        report.clone(),
        class,
        desired.clone(),
    )?;
    Ok(desired)
}

fn require_authority_inputs(
    saved_binding: &RunBindingReceipt,
    saved_plan: &RunTerminalPlan,
    saved_intent: &RunTerminalIntent,
    binding: &RunBindingReceipt,
    plan: &RunTerminalPlan,
    intent: &RunTerminalIntent,
) -> Result<(), TuneError> {
    if saved_binding != binding || saved_plan != plan || saved_intent != intent {
        return Err(authority_mismatch());
    }
    Ok(())
}

fn require_saved_report(
    saved_binding: &RunBindingReceipt,
    saved_report: &RunTerminalReport,
    saved_class: RunTerminalClass,
    binding: &RunBindingReceipt,
    report: &RunTerminalReport,
    class: RunTerminalClass,
) -> Result<(), TuneError> {
    if saved_binding != binding || saved_report != report || saved_class != class {
        return Err(authority_mismatch());
    }
    Ok(())
}

fn recover_authoritative_receipt_blocking<V>(
    journal: &Journal,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    binding: &RunBindingReceipt,
    plan: &RunTerminalPlan,
    desired: &RunTerminalReceipt,
) -> Result<Option<RunTerminalReceipt>, TuneError>
where
    V: RunTerminalAdapter,
{
    let receipts = recover_receipts_blocking(journal, vehicle, capability, binding, plan)?;
    validate_returned_receipts(journal, binding, &receipts)?;
    match receipts.as_slice() {
        [] => Ok(None),
        [receipt] if receipt == desired => Ok(Some(receipt.clone())),
        [_] => Err(poison_receipt_set(
            journal,
            "the recovered terminal receipt differs from durable authority",
        )),
        _ => Err(poison_receipt_set(
            journal,
            "the adapter returned more than one terminal receipt",
        )),
    }
}

enum Publication {
    Exact(Box<RunTerminalReceipt>),
    AbsentAfterFailure(TuneError),
    AbsentAfterSuccess,
}

fn publish_and_read_blocking<V>(
    journal: &Journal,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    binding: &RunBindingReceipt,
    plan: &RunTerminalPlan,
    desired: &RunTerminalReceipt,
) -> Result<Publication, TuneError>
where
    V: RunTerminalAdapter,
{
    let publication = seal_receipt_blocking(journal, vehicle, capability, binding, plan, desired);
    let receipts = recover_receipts_blocking(journal, vehicle, capability, binding, plan)?;
    validate_returned_receipts(journal, binding, &receipts)?;
    match receipts.as_slice() {
        [receipt] if receipt == desired => Ok(Publication::Exact(Box::new(receipt.clone()))),
        [] => match publication {
            Ok(()) => Ok(Publication::AbsentAfterSuccess),
            Err(error) => Ok(Publication::AbsentAfterFailure(error)),
        },
        [_] => Err(poison_receipt_set(
            journal,
            "terminal receipt readback changed after publication",
        )),
        _ => Err(poison_receipt_set(
            journal,
            "terminal receipt publication produced conflicting receipts",
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn finish_publication_blocking<V>(
    journal: &mut Journal,
    trial_id: u64,
    run_index: u64,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    binding: &RunBindingReceipt,
    plan: &RunTerminalPlan,
    desired: &RunTerminalReceipt,
    publication: Publication,
) -> Result<RunTerminalReceipt, TuneError>
where
    V: RunTerminalAdapter,
{
    match publication {
        Publication::Exact(receipt) => commit_receipt(journal, trial_id, run_index, *receipt),
        Publication::AbsentAfterSuccess => Err(missing_after_success()),
        Publication::AbsentAfterFailure(error) if !desired.class().is_completed() => Err(error),
        Publication::AbsentAfterFailure(_) => publish_evidence_failure_blocking(
            journal, trial_id, run_index, vehicle, capability, binding, plan, desired,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn publish_evidence_failure_blocking<V>(
    journal: &mut Journal,
    trial_id: u64,
    run_index: u64,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    binding: &RunBindingReceipt,
    plan: &RunTerminalPlan,
    desired: &RunTerminalReceipt,
) -> Result<RunTerminalReceipt, TuneError>
where
    V: RunTerminalAdapter,
{
    let intent = desired.intent();
    let report = desired.report();
    let class = RunTerminalClass::evidence_failure(intent, report)?;
    journal.record_run_terminal_evidence_failure(trial_id, run_index, class)?;
    let quarantine = RunTerminalReceipt::new(
        binding,
        intent,
        report,
        class,
        desired.causal_evidence_digest(),
    )?;
    match publish_and_read_blocking(journal, vehicle, capability, binding, plan, &quarantine)? {
        Publication::Exact(receipt) => commit_receipt(journal, trial_id, run_index, *receipt),
        Publication::AbsentAfterFailure(error) => Err(error),
        Publication::AbsentAfterSuccess => Err(missing_after_success()),
    }
}

fn recover_receipts_blocking<V>(
    journal: &Journal,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    binding: &RunBindingReceipt,
    plan: &RunTerminalPlan,
) -> Result<Vec<RunTerminalReceipt>, TuneError>
where
    V: RunTerminalAdapter,
{
    journal.ensure_usable()?;
    let result = vehicle.recover_terminal_receipts_blocking(capability, binding, plan);
    journal.ensure_usable()?;
    result.map_err(|source| terminal_error("recover terminal receipts", source))
}

fn validate_returned_receipts(
    journal: &Journal,
    binding: &RunBindingReceipt,
    receipts: &[RunTerminalReceipt],
) -> Result<(), TuneError> {
    if receipts
        .iter()
        .any(|receipt| receipt.validate().is_err() || receipt.binding() != binding)
    {
        return Err(poison_receipt_set(
            journal,
            "the adapter returned a malformed or foreign terminal receipt",
        ));
    }
    Ok(())
}

fn seal_receipt_blocking<V>(
    journal: &Journal,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    binding: &RunBindingReceipt,
    plan: &RunTerminalPlan,
    receipt: &RunTerminalReceipt,
) -> Result<(), TuneError>
where
    V: RunTerminalAdapter,
{
    journal.ensure_usable()?;
    let result = vehicle.seal_terminal_receipt_blocking(capability, binding, plan, receipt);
    journal.ensure_usable()?;
    result.map_err(|source| terminal_error("seal terminal receipt", source))
}

fn commit_receipt(
    journal: &mut Journal,
    trial_id: u64,
    run_index: u64,
    receipt: RunTerminalReceipt,
) -> Result<RunTerminalReceipt, TuneError> {
    journal.commit_run(trial_id, run_index, receipt.clone())?;
    Ok(receipt)
}

fn poison_receipt_set(journal: &Journal, detail: &str) -> TuneError {
    journal.poison();
    TuneError::ReceiptMismatch {
        operation: "recover terminal receipt",
        detail: detail.to_owned(),
    }
}

fn missing_authority_state() -> TuneError {
    TuneError::InvalidState {
        operation: "resolve terminal receipt authority",
        detail: "the current run has no terminal report authority state".to_owned(),
    }
}

fn authority_mismatch() -> TuneError {
    TuneError::ReceiptMismatch {
        operation: "resolve terminal receipt authority",
        detail: "the terminal report arguments differ from durable authority".to_owned(),
    }
}

fn missing_after_success() -> TuneError {
    TuneError::InvalidState {
        operation: "seal terminal receipt",
        detail: "the adapter acknowledged the seal but returned no durable receipt".to_owned(),
    }
}
