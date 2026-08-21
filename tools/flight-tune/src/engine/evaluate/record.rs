use crate::journal::{AttemptRole, OperationStatus};
use crate::score::aggregate_runs;
use crate::{
    CandidateEvaluation, GateEvaluator, Journal, MetricEvaluator, RunRecord, SearchStage,
    SimulatorBackend, TuneError,
};

use super::contract::{RunContext, RunTerminal, adapter_error, run_record, training_selection};
use super::{finish_cleanup, quarantine_after_error};

pub(super) enum RunProgress {
    Continue,
    Complete,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn record_terminal<B, G, M>(
    journal: &mut Journal,
    trial_id: u64,
    role: AttemptRole,
    stage: &SearchStage,
    context: &RunContext<'_>,
    expected_runs: usize,
    runs: &mut Vec<RunRecord>,
    terminal: RunTerminal,
    backend: &mut B,
    gates: &mut G,
    metric: &mut M,
) -> Result<RunProgress, TuneError>
where
    B: SimulatorBackend,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    match terminal {
        RunTerminal::Passed { values, stop } => record_passed_run(
            journal,
            trial_id,
            role,
            stage,
            context,
            expected_runs,
            runs,
            values,
            stop,
            backend,
            gates,
            metric,
        ),
        RunTerminal::HardGate { failure } => {
            let evaluation = CandidateEvaluation::HardGateFailed {
                failure,
                completed_runs: std::mem::take(runs),
            };
            let selected = training_selection(journal, role, &evaluation);
            journal.complete_attempt(trial_id, evaluation, selected)?;
            finish_cleanup(
                journal,
                trial_id,
                OperationStatus::NotRequired,
                backend,
                Some(()),
                gates,
                metric,
            )?;
            Ok(RunProgress::Complete)
        }
        RunTerminal::Failed { error, started } => quarantine_after_error(
            journal,
            trial_id,
            error,
            OperationStatus::NotRequired,
            backend,
            started.then_some(()),
            gates,
            metric,
        )
        .map(|()| RunProgress::Complete),
    }
}

#[allow(clippy::too_many_arguments)]
fn record_passed_run<B, G, M>(
    journal: &mut Journal,
    trial_id: u64,
    role: AttemptRole,
    stage: &SearchStage,
    context: &RunContext<'_>,
    expected_runs: usize,
    runs: &mut Vec<RunRecord>,
    values: crate::MetricValues,
    stop: OperationStatus,
    backend: &mut B,
    gates: &mut G,
    metric: &mut M,
) -> Result<RunProgress, TuneError>
where
    B: SimulatorBackend,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    runs.push(run_record(stage, context, values));
    if runs.len() < expected_runs {
        if let Err(source) = backend.cleanup_blocking() {
            let error = adapter_error(backend, "cleanup", source);
            return quarantine_after_error(
                journal, trial_id, error, stop, backend, None, gates, metric,
            )
            .map(|()| RunProgress::Complete);
        }
        return Ok(RunProgress::Continue);
    }
    let aggregate = match aggregate_runs(runs, context.set) {
        Ok(aggregate) => aggregate,
        Err(error) => {
            return quarantine_after_error(
                journal, trial_id, error, stop, backend, None, gates, metric,
            )
            .map(|()| RunProgress::Complete);
        }
    };
    let evaluation = CandidateEvaluation::Passed {
        aggregate,
        runs: std::mem::take(runs),
    };
    let selected = training_selection(journal, role, &evaluation);
    journal.complete_attempt(trial_id, evaluation, selected)?;
    finish_cleanup(journal, trial_id, stop, backend, None, gates, metric)?;
    Ok(RunProgress::Complete)
}
