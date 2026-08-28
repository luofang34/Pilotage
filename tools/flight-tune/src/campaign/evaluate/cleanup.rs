use crate::journal::OperationStatus;
use crate::{CampaignBackend, GateEvaluator, Journal, MetricEvaluator, TuneError};

pub(super) fn finish_cleanup<B, G, M>(
    journal: &mut Journal,
    trial_id: u64,
    backend: &mut B,
    gates: &mut G,
    metric: &mut M,
    cancel_evaluators: bool,
) -> Result<(), TuneError>
where
    B: CampaignBackend,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    journal.ensure_usable()?;
    let cleanup = cleanup_status(journal, backend, gates, metric, cancel_evaluators)?;
    let succeeded = cleanup.succeeded();
    journal.record_cleanup(trial_id, cleanup)?;
    if succeeded {
        Ok(())
    } else {
        Err(TuneError::InvalidState {
            operation: "cleanup",
            detail: "the simulator or evaluator cleanup failed".to_owned(),
        })
    }
}

pub(super) fn cleanup_status<B, G, M>(
    journal: &Journal,
    backend: &mut B,
    gates: &mut G,
    metric: &mut M,
    cancel_evaluators: bool,
) -> Result<OperationStatus, TuneError>
where
    B: CampaignBackend,
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
