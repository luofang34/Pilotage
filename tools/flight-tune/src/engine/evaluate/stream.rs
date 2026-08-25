use std::time::Duration;

use crate::score::{validate_gate_outcomes, validate_metric};
use crate::{
    GateEvaluator, GateOutcome, HardGateFailure, Journal, MetricEvaluator, OperationStatus,
    SampleEvent, SearchStage, SimulatorBackend, TelemetrySample, TuneError,
};

use super::contract::{RunContext, RunTerminal, adapter_error, evaluator_error, validate_sample};

pub(super) fn stream_samples<B, G, M>(
    journal: &Journal,
    stage: &SearchStage,
    context: &RunContext<'_>,
    backend: &mut B,
    gates: &mut G,
    metric: &mut M,
) -> RunTerminal
where
    B: SimulatorBackend,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    let timeout = Duration::from_millis(u64::from(context.scenario.sample_timeout_ms));
    let mut position = StreamPosition::default();
    loop {
        if let Err(error) = journal.ensure_usable() {
            return RunTerminal::Failed {
                error,
                started: true,
            };
        }
        match backend.sample_blocking(timeout) {
            Ok(SampleEvent::Sample(sample)) => {
                if let Some(terminal) = process_sample(
                    journal,
                    stage,
                    context,
                    sample,
                    &mut position,
                    gates,
                    metric,
                ) {
                    return terminal;
                }
            }
            Ok(SampleEvent::Complete) => {
                return finish_stream_blocking(
                    journal,
                    context,
                    position.expected_sequence,
                    backend,
                    gates,
                    metric,
                );
            }
            Ok(SampleEvent::TimedOut) => {
                return hard_failure(
                    context,
                    position.expected_sequence,
                    position.elapsed_ms,
                    GateOutcome::fail(
                        "core.sample_timeout",
                        "the simulator did not supply a sample before the timeout",
                    ),
                );
            }
            Err(source) => {
                return RunTerminal::Failed {
                    error: adapter_error(backend, "sample", source),
                    started: true,
                };
            }
        }
    }
}

#[derive(Default)]
struct StreamPosition {
    expected_sequence: u64,
    elapsed_ms: u64,
}

#[allow(clippy::too_many_arguments)]
fn process_sample<G, M>(
    journal: &Journal,
    stage: &SearchStage,
    context: &RunContext<'_>,
    sample: TelemetrySample,
    position: &mut StreamPosition,
    gates: &mut G,
    metric: &mut M,
) -> Option<RunTerminal>
where
    G: GateEvaluator,
    M: MetricEvaluator,
{
    if position.expected_sequence >= u64::from(context.scenario.max_samples) {
        return Some(hard_failure(
            context,
            sample.sequence,
            sample.elapsed_ms,
            GateOutcome::fail(
                "core.sample_limit",
                "the simulator exceeded the scenario sample limit",
            ),
        ));
    }
    if let Err(error) = validate_sample(&sample, position.expected_sequence, position.elapsed_ms) {
        return Some(RunTerminal::Failed {
            error,
            started: true,
        });
    }
    position.elapsed_ms = sample.elapsed_ms;
    position.expected_sequence = position.expected_sequence.wrapping_add(1);
    match evaluate_sample(journal, stage, &sample, gates, metric) {
        Ok(Some(gate)) => Some(hard_failure(
            context,
            sample.sequence,
            sample.elapsed_ms,
            gate,
        )),
        Ok(None) => None,
        Err(error) => Some(RunTerminal::Failed {
            error,
            started: true,
        }),
    }
}

fn finish_stream_blocking<B, G, M>(
    journal: &Journal,
    context: &RunContext<'_>,
    sample_count: u64,
    backend: &mut B,
    gates: &mut G,
    metric: &mut M,
) -> RunTerminal
where
    B: SimulatorBackend,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    if sample_count == 0 {
        return hard_failure(
            context,
            0,
            0,
            GateOutcome::fail(
                "core.no_samples",
                "the simulator completed without telemetry samples",
            ),
        );
    }
    finish_normal_run_blocking(journal, backend, gates, metric)
}

fn evaluate_sample<G, M>(
    journal: &Journal,
    stage: &SearchStage,
    sample: &TelemetrySample,
    gates: &mut G,
    metric: &mut M,
) -> Result<Option<GateOutcome>, TuneError>
where
    G: GateEvaluator,
    M: MetricEvaluator,
{
    journal.ensure_usable()?;
    let outcomes = gates
        .evaluate(sample)
        .map_err(|source| evaluator_error(gates.identity(), "evaluate hard gates", source))?;
    if let Some(failure) = validate_gate_outcomes(&stage.required_hard_gates, &outcomes)? {
        return Ok(Some(failure));
    }
    journal.ensure_usable()?;
    metric
        .observe(sample)
        .map_err(|source| evaluator_error(metric.identity(), "observe metric", source))?;
    Ok(None)
}

fn finish_normal_run_blocking<B, G, M>(
    journal: &Journal,
    backend: &mut B,
    gates: &mut G,
    metric: &mut M,
) -> RunTerminal
where
    B: SimulatorBackend,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    if let Err(error) = journal.ensure_usable() {
        return RunTerminal::Failed {
            error,
            started: true,
        };
    }
    if let Err(source) = backend.stop_blocking() {
        return RunTerminal::Failed {
            error: adapter_error(backend, "stop", source),
            started: true,
        };
    }
    if let Err(error) = journal.ensure_usable() {
        return RunTerminal::Failed {
            error,
            started: false,
        };
    }
    if let Err(source) = gates.finish() {
        return RunTerminal::Failed {
            error: evaluator_error(gates.identity(), "finish hard gates", source),
            started: false,
        };
    }
    if let Err(error) = journal.ensure_usable() {
        return RunTerminal::Failed {
            error,
            started: false,
        };
    }
    let values = match metric.finish() {
        Ok(values) => values,
        Err(source) => {
            return RunTerminal::Failed {
                error: evaluator_error(metric.identity(), "finish metric", source),
                started: false,
            };
        }
    };
    if let Err(error) = validate_metric(&values) {
        return RunTerminal::Failed {
            error,
            started: false,
        };
    }
    RunTerminal::Passed {
        values,
        stop: OperationStatus::Succeeded,
    }
}

fn hard_failure(
    context: &RunContext<'_>,
    sample_sequence: u64,
    elapsed_ms: u64,
    gate: GateOutcome,
) -> RunTerminal {
    RunTerminal::HardGate {
        failure: HardGateFailure {
            scenario_set: context.set,
            scenario_id: context.scenario.id.clone(),
            repetition: context.repetition,
            seed: context.seed,
            sample_sequence,
            elapsed_ms,
            gate,
        },
    }
}
