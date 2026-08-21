mod contract;
mod record;

use std::time::Duration;

use crate::identity::digest_bytes;
use crate::journal::{AttemptRole, OperationStatus};
use crate::model::derive_seed;
use crate::score::{validate_gate_outcomes, validate_metric};
use crate::{
    Candidate, Digest, GateEvaluator, GateOutcome, HardGateFailure, Journal, MetricEvaluator,
    SampleEvent, SearchStage, SimulatorBackend, SimulatorCapability, SimulatorVehicleAdapter,
    TelemetrySample, TuneError, VehicleBinding,
};
use contract::*;
use record::{RunProgress, record_terminal};

pub(super) fn plan_digest(
    stage: &SearchStage,
    role: AttemptRole,
    candidate: Digest,
    fixed_seed: u64,
) -> Result<Digest, TuneError> {
    role.plan_digest(stage, candidate, fixed_seed)
}

pub(super) fn candidate_digest(candidate: &Candidate) -> Result<Digest, TuneError> {
    let bytes = serde_json::to_vec(candidate).map_err(|source| TuneError::Encode {
        document: "candidate",
        source,
    })?;
    Ok(digest_bytes(&bytes))
}

pub(super) fn recover_pending_blocking<B, G, M>(
    journal: &mut Journal,
    backend: &mut B,
    gates: &mut G,
    metric: &mut M,
) -> Result<(), TuneError>
where
    B: SimulatorBackend,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    let Some(pending) = journal.state().pending.clone() else {
        return Ok(());
    };
    if pending.outcome.is_none() {
        journal.quarantine_attempt(
            pending.trial_id,
            "process stopped after AttemptPrepared; automatic replay is forbidden",
        )?;
    }
    let stop = operation_status(|| backend.stop_blocking());
    let cleanup = cleanup_status(backend, gates, metric, true);
    journal.record_cleanup(pending.trial_id, stop, cleanup.clone())?;
    if cleanup.succeeded() {
        Ok(())
    } else {
        Err(TuneError::InvalidState {
            operation: "recover pending attempt",
            detail: "cleanup did not restore an idle simulator".to_owned(),
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_prepared_blocking<B, V, G, M>(
    journal: &mut Journal,
    stage: &SearchStage,
    trial_id: u64,
    role: AttemptRole,
    candidate: &Candidate,
    candidate_digest: Digest,
    backend: &mut B,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    gates: &mut G,
    metric: &mut M,
) -> Result<(), TuneError>
where
    B: SimulatorBackend,
    V: SimulatorVehicleAdapter,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    let set = role.scenario_set();
    let scenario_count = scenarios(stage, set).len();
    let expected_runs = scenario_count * stage.repetitions as usize;
    let mut runs = Vec::new();
    for scenario in scenarios(stage, set) {
        for repetition in 0..stage.repetitions {
            let seed = derive_seed(journal.session().fixed_seed, set, scenario, repetition);
            let context = RunContext {
                set,
                scenario,
                repetition,
                seed,
            };
            let terminal = run_until_terminal(
                stage,
                &context,
                candidate,
                candidate_digest,
                backend,
                vehicle,
                capability,
                gates,
                metric,
            );
            let progress = record_terminal(
                journal,
                trial_id,
                role,
                stage,
                &context,
                expected_runs,
                &mut runs,
                terminal,
                backend,
                gates,
                metric,
            )?;
            if matches!(progress, RunProgress::Complete) {
                return Ok(());
            }
        }
    }
    Err(TuneError::InvalidState {
        operation: "evaluate candidate",
        detail: "the run plan produced no result".to_owned(),
    })
}

#[allow(clippy::too_many_arguments)]
fn run_until_terminal<B, V, G, M>(
    stage: &SearchStage,
    context: &RunContext<'_>,
    candidate: &Candidate,
    candidate_digest: Digest,
    backend: &mut B,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    gates: &mut G,
    metric: &mut M,
) -> RunTerminal
where
    B: SimulatorBackend,
    V: SimulatorVehicleAdapter,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    if let Err(source) = backend.prepare_blocking(capability, context.scenario, context.seed) {
        return RunTerminal::Failed {
            error: adapter_error(backend, "prepare", source),
            started: false,
        };
    }
    let receipt =
        vehicle
            .adapter_mut()
            .apply_candidate_blocking(capability, candidate, candidate_digest);
    if let Err(error) = validate_candidate_receipt(receipt, capability, candidate_digest) {
        return RunTerminal::Failed {
            error,
            started: false,
        };
    }
    if let Err(error) = begin_evaluators(context.scenario, gates, metric) {
        return RunTerminal::Failed {
            error,
            started: false,
        };
    }
    let receipt = match backend.start_blocking(capability) {
        Ok(receipt) => receipt,
        Err(source) => {
            return RunTerminal::Failed {
                error: adapter_error(backend, "start", source),
                started: true,
            };
        }
    };
    if let Err(error) = validate_scenario_receipt(receipt, capability, context) {
        return RunTerminal::Failed {
            error,
            started: true,
        };
    }
    stream_samples(stage, context, backend, gates, metric)
}

fn stream_samples<B, G, M>(
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
    let mut expected_sequence = 0_u64;
    let mut elapsed_ms = 0_u64;
    loop {
        match backend.sample_blocking(timeout) {
            Ok(SampleEvent::Sample(sample)) => {
                if expected_sequence >= u64::from(context.scenario.max_samples) {
                    return hard_failure(
                        context,
                        sample.sequence,
                        sample.elapsed_ms,
                        GateOutcome::fail(
                            "core.sample_limit",
                            "the simulator exceeded the scenario sample limit",
                        ),
                    );
                }
                if let Err(error) = validate_sample(&sample, expected_sequence, elapsed_ms) {
                    return RunTerminal::Failed {
                        error,
                        started: true,
                    };
                }
                elapsed_ms = sample.elapsed_ms;
                expected_sequence = expected_sequence.wrapping_add(1);
                match evaluate_sample(stage, &sample, gates, metric) {
                    Ok(Some(gate)) => {
                        return hard_failure(context, sample.sequence, sample.elapsed_ms, gate);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        return RunTerminal::Failed {
                            error,
                            started: true,
                        };
                    }
                }
            }
            Ok(SampleEvent::Complete) => {
                return finish_normal_run(backend, gates, metric);
            }
            Ok(SampleEvent::TimedOut) => {
                return hard_failure(
                    context,
                    expected_sequence,
                    elapsed_ms,
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

fn evaluate_sample<G, M>(
    stage: &SearchStage,
    sample: &TelemetrySample,
    gates: &mut G,
    metric: &mut M,
) -> Result<Option<GateOutcome>, TuneError>
where
    G: GateEvaluator,
    M: MetricEvaluator,
{
    let outcomes = gates
        .evaluate(sample)
        .map_err(|source| evaluator_error(gates.identity(), "evaluate hard gates", source))?;
    if let Some(failure) = validate_gate_outcomes(&stage.required_hard_gates, &outcomes)? {
        return Ok(Some(failure));
    }
    metric
        .observe(sample)
        .map_err(|source| evaluator_error(metric.identity(), "observe metric", source))?;
    Ok(None)
}

fn finish_normal_run<B, G, M>(backend: &mut B, gates: &mut G, metric: &mut M) -> RunTerminal
where
    B: SimulatorBackend,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    if let Err(source) = backend.stop_blocking() {
        return RunTerminal::Failed {
            error: adapter_error(backend, "stop", source),
            started: true,
        };
    }
    if let Err(source) = gates.finish() {
        return RunTerminal::Failed {
            error: evaluator_error(gates.identity(), "finish hard gates", source),
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
    if let Err(error) = validate_metric(values) {
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

fn finish_cleanup<B, G, M>(
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
    if stop_required.is_some() {
        stop = operation_status(|| backend.stop_blocking());
    }
    let cleanup = cleanup_status(backend, gates, metric, stop_required.is_some());
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
fn quarantine_after_error<B, G, M>(
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
    journal.quarantine_attempt(trial_id, error.to_string())?;
    if stop_required.is_some() {
        stop = operation_status(|| backend.stop_blocking());
    }
    let cleanup = cleanup_status(backend, gates, metric, true);
    journal.record_cleanup(trial_id, stop, cleanup)?;
    Err(error)
}

fn cleanup_status<B, G, M>(
    backend: &mut B,
    gates: &mut G,
    metric: &mut M,
    cancel_evaluators: bool,
) -> OperationStatus
where
    B: SimulatorBackend,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    let mut failures = Vec::new();
    if cancel_evaluators {
        if let Err(error) = gates.cancel() {
            failures.push(format!("hard gate cancel: {error}"));
        }
        if let Err(error) = metric.cancel() {
            failures.push(format!("metric cancel: {error}"));
        }
    }
    if let Err(error) = backend.cleanup_blocking() {
        failures.push(format!("simulator cleanup: {error}"));
    }
    if failures.is_empty() {
        OperationStatus::Succeeded
    } else {
        OperationStatus::Failed {
            detail: failures.join("; "),
        }
    }
}

fn operation_status(
    operation: impl FnOnce() -> Result<(), crate::AdapterError>,
) -> OperationStatus {
    match operation() {
        Ok(()) => OperationStatus::Succeeded,
        Err(error) => OperationStatus::Failed {
            detail: error.to_string(),
        },
    }
}
