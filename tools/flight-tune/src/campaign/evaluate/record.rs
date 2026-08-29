use crate::journal::AttemptRole;
use crate::score::aggregate_runs;
use crate::{
    CampaignBackend, CandidateEvaluation, GateEvaluator, HardGateFailure, Journal, MetricEvaluator,
    RunRecord, RunTerminalCompletion, RunTerminalDisposition, RunTerminalReceipt,
    RunTerminalSemanticOutcome, TuneError,
};

use super::cleanup::{cleanup_status, finish_cleanup};

pub(super) enum RunProgress {
    Continue,
    Complete,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn record_committed_terminal<B, G, M>(
    journal: &mut Journal,
    trial_id: u64,
    role: AttemptRole,
    expected_runs: usize,
    runs: &mut Vec<RunRecord>,
    receipt: &RunTerminalReceipt,
    primary_error: Option<TuneError>,
    backend: &mut B,
    gates: &mut G,
    metric: &mut M,
) -> Result<RunProgress, TuneError>
where
    B: CampaignBackend,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    receipt.validate()?;
    match (receipt.class().disposition(), receipt.intent().outcome()) {
        (
            RunTerminalDisposition::Completed {
                completion: RunTerminalCompletion::ScenarioComplete,
            },
            RunTerminalSemanticOutcome::ScenarioComplete { run, .. },
        ) => record_completed_run(
            journal,
            trial_id,
            role,
            expected_runs,
            runs,
            run.clone(),
            backend,
            gates,
            metric,
        ),
        (
            RunTerminalDisposition::Completed {
                completion: RunTerminalCompletion::HardGateAbort,
            },
            RunTerminalSemanticOutcome::HardGateAbort { failure, .. },
        ) => record_hard_gate(
            journal,
            trial_id,
            role,
            runs,
            failure.clone(),
            backend,
            gates,
            metric,
        ),
        (RunTerminalDisposition::Quarantine { .. }, _) => {
            quarantine_terminal(journal, trial_id, primary_error, backend, gates, metric)
        }
        _ => Err(TuneError::InvalidState {
            operation: "record committed run",
            detail: "the receipt class and semantic result do not match".to_owned(),
        }),
    }
}

pub(super) fn completed_scenario_run(receipt: &RunTerminalReceipt) -> Result<RunRecord, TuneError> {
    receipt.validate()?;
    match (receipt.class().disposition(), receipt.intent().outcome()) {
        (
            RunTerminalDisposition::Completed {
                completion: RunTerminalCompletion::ScenarioComplete,
            },
            RunTerminalSemanticOutcome::ScenarioComplete { run, .. },
        ) => Ok(run.clone()),
        _ => Err(TuneError::InvalidState {
            operation: "resume committed run",
            detail: "a committed prefix contains a terminal attempt result".to_owned(),
        }),
    }
}

#[allow(clippy::too_many_arguments)]
fn record_completed_run<B, G, M>(
    journal: &mut Journal,
    trial_id: u64,
    role: AttemptRole,
    expected_runs: usize,
    runs: &mut Vec<RunRecord>,
    run: RunRecord,
    backend: &mut B,
    gates: &mut G,
    metric: &mut M,
) -> Result<RunProgress, TuneError>
where
    B: CampaignBackend,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    runs.push(run);
    if runs.len() < expected_runs {
        let cleanup = cleanup_status(journal, backend, gates, metric, false)?;
        if cleanup.succeeded() {
            return Ok(RunProgress::Continue);
        }
        return Err(TuneError::InvalidState {
            operation: "prepare next run",
            detail: "the simulator cleanup failed after a committed run".to_owned(),
        });
    }
    let aggregate = aggregate_runs(runs, role.scenario_set())?;
    let evaluation = CandidateEvaluation::Passed {
        aggregate,
        runs: std::mem::take(runs),
    };
    complete_and_clean(
        journal, trial_id, role, evaluation, backend, gates, metric, false,
    )?;
    Ok(RunProgress::Complete)
}

#[allow(clippy::too_many_arguments)]
fn record_hard_gate<B, G, M>(
    journal: &mut Journal,
    trial_id: u64,
    role: AttemptRole,
    runs: &mut Vec<RunRecord>,
    failure: HardGateFailure,
    backend: &mut B,
    gates: &mut G,
    metric: &mut M,
) -> Result<RunProgress, TuneError>
where
    B: CampaignBackend,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    let evaluation = CandidateEvaluation::HardGateFailed {
        failure,
        completed_runs: std::mem::take(runs),
    };
    complete_and_clean(
        journal, trial_id, role, evaluation, backend, gates, metric, true,
    )?;
    Ok(RunProgress::Complete)
}

#[allow(clippy::too_many_arguments)]
fn complete_and_clean<B, G, M>(
    journal: &mut Journal,
    trial_id: u64,
    role: AttemptRole,
    evaluation: CandidateEvaluation,
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
    let selected = journal.training_selection(role, &evaluation)?;
    journal.complete_attempt(trial_id, evaluation, selected)?;
    finish_cleanup(journal, trial_id, backend, gates, metric, cancel_evaluators)
}

fn quarantine_terminal<B, G, M>(
    journal: &mut Journal,
    trial_id: u64,
    primary_error: Option<TuneError>,
    backend: &mut B,
    gates: &mut G,
    metric: &mut M,
) -> Result<RunProgress, TuneError>
where
    B: CampaignBackend,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    if let Err(error) = journal.quarantine_attempt(trial_id) {
        return Err(primary_error.unwrap_or(error));
    }
    // The decision is durable before cleanup, so a session that stops in the
    // cleanup window cannot leave a quarantine a later session is free to
    // answer either way.
    if let Err(error) = journal.record_retry_decision(trial_id) {
        return Err(primary_error.unwrap_or(error));
    }
    let cleanup = finish_cleanup(journal, trial_id, backend, gates, metric, true);
    if let Some(error) = primary_error {
        return Err(error);
    }
    cleanup?;
    Ok(RunProgress::Complete)
}
