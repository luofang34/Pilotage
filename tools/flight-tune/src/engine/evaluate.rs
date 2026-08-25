mod cleanup;
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
use cleanup::{cleanup_status, finish_cleanup, operation_status, quarantine_after_error};
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

pub(super) fn ensure_candidate_blocking<V>(
    journal: &Journal,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    candidate: &Candidate,
    candidate_digest: Digest,
) -> Result<(), TuneError>
where
    V: SimulatorVehicleAdapter,
{
    journal.ensure_usable()?;
    let receipt =
        vehicle
            .adapter_mut()
            .ensure_candidate_blocking(capability, candidate, candidate_digest);
    validate_candidate_receipt(receipt, capability, candidate_digest)?;
    journal.ensure_usable()
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
    journal.ensure_usable()?;
    let Some(pending) = journal.state().pending.clone() else {
        return Ok(());
    };
    if pending.outcome.is_none() {
        journal.quarantine_attempt(
            pending.trial_id,
            "process stopped after AttemptPrepared; automatic replay is forbidden",
        )?;
    }
    journal.ensure_usable()?;
    let stop = operation_status(|| backend.stop_blocking());
    let cleanup = cleanup_status(journal, backend, gates, metric, true)?;
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
    journal.ensure_usable()?;
    let set = role.scenario_set();
    let scenario_count = scenarios(stage, set).len();
    let expected_runs = scenario_count * stage.repetitions as usize;
    let mut runs = Vec::new();
    for scenario in scenarios(stage, set) {
        for repetition in 0..stage.repetitions {
            journal.ensure_usable()?;
            let seed = derive_seed(journal.session().fixed_seed, set, scenario, repetition);
            let context = RunContext {
                set,
                scenario,
                repetition,
                seed,
            };
            let terminal = run_until_terminal(
                journal,
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
    journal: &Journal,
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
    if let Err(error) = journal.ensure_usable() {
        return RunTerminal::Failed {
            error,
            started: false,
        };
    }
    if let Err(source) = backend.prepare_blocking(capability, context.scenario, context.seed) {
        return RunTerminal::Failed {
            error: adapter_error(backend, "prepare", source),
            started: false,
        };
    }
    if let Err(error) =
        ensure_candidate_blocking(journal, vehicle, capability, candidate, candidate_digest)
    {
        return RunTerminal::Failed {
            error,
            started: false,
        };
    }
    if let Err(error) = begin_evaluators(journal, context.scenario, gates, metric) {
        return RunTerminal::Failed {
            error,
            started: false,
        };
    }
    if let Err(error) = journal.ensure_usable() {
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
    stream_samples(journal, stage, context, backend, gates, metric)
}

fn stream_samples<B, G, M>(
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
