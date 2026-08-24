use crate::journal::OperationStatus;
use crate::{GateEvaluator, Journal, MetricEvaluator, SimulatorBackend, TuneError};

pub(super) fn finish_cleanup<B, G, M>(
    journal: &mut Journal,
    trial_id: u64,
    mut stop: OperationStatus,
    backend: &mut B,
    stop_required: Option<()>,
    gates: &mut G,
    metric: &mut M,
) -> Result<(), TuneError>
where
    B: SimulatorBackend,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    journal.ensure_usable()?;
    if stop_required.is_some() {
        stop = authorized_operation_status(journal, || backend.stop_blocking())?;
    }
    let cleanup = cleanup_status(journal, backend, gates, metric, stop_required.is_some())?;
    let succeeded = stop.succeeded() && cleanup.succeeded();
    journal.record_cleanup(trial_id, stop, cleanup.clone())?;
    if succeeded {
        Ok(())
    } else {
        Err(TuneError::InvalidState {
            operation: "cleanup",
            detail: "the simulator or evaluator is not clean".to_owned(),
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn quarantine_after_error<B, G, M>(
    journal: &mut Journal,
    trial_id: u64,
    error: TuneError,
    mut stop: OperationStatus,
    backend: &mut B,
    stop_required: Option<()>,
    gates: &mut G,
    metric: &mut M,
) -> Result<(), TuneError>
where
    B: SimulatorBackend,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    if let Err(journal_error) = journal.quarantine_attempt(trial_id, error.to_string()) {
        return preserve_primary(journal, error, journal_error);
    }
    journal.ensure_usable()?;
    if stop_required.is_some() {
        stop = match authorized_operation_status(journal, || backend.stop_blocking()) {
            Ok(status) => status,
            Err(cleanup_error) => return preserve_primary(journal, error, cleanup_error),
        };
    }
    let cleanup = match cleanup_status(journal, backend, gates, metric, true) {
        Ok(status) => status,
        Err(cleanup_error) => return preserve_primary(journal, error, cleanup_error),
    };
    if let Err(journal_error) = journal.record_cleanup(trial_id, stop, cleanup) {
        return preserve_primary(journal, error, journal_error);
    }
    Err(error)
}

pub(super) fn cleanup_status<B, G, M>(
    journal: &Journal,
    backend: &mut B,
    gates: &mut G,
    metric: &mut M,
    cancel_evaluators: bool,
) -> Result<OperationStatus, TuneError>
where
    B: SimulatorBackend,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    let mut failures = Vec::new();
    if cancel_evaluators {
        journal.ensure_usable()?;
        if let Err(error) = gates.cancel() {
            failures.push(format!("hard gate cancel: {error}"));
        }
        journal.ensure_usable()?;
        if let Err(error) = metric.cancel() {
            failures.push(format!("metric cancel: {error}"));
        }
    }
    journal.ensure_usable()?;
    if let Err(error) = backend.cleanup_blocking() {
        failures.push(format!("simulator cleanup: {error}"));
    }
    if failures.is_empty() {
        Ok(OperationStatus::Succeeded)
    } else {
        Ok(OperationStatus::Failed {
            detail: failures.join("; "),
        })
    }
}

pub(super) fn operation_status(
    operation: impl FnOnce() -> Result<(), crate::AdapterError>,
) -> OperationStatus {
    match operation() {
        Ok(()) => OperationStatus::Succeeded,
        Err(error) => OperationStatus::Failed {
            detail: error.to_string(),
        },
    }
}

fn authorized_operation_status(
    journal: &Journal,
    operation: impl FnOnce() -> Result<(), crate::AdapterError>,
) -> Result<OperationStatus, TuneError> {
    journal.ensure_usable()?;
    Ok(operation_status(operation))
}

fn preserve_primary(
    journal: &Journal,
    primary: TuneError,
    secondary: TuneError,
) -> Result<(), TuneError> {
    if journal.ensure_usable().is_err() {
        Err(primary)
    } else {
        Err(secondary)
    }
}
