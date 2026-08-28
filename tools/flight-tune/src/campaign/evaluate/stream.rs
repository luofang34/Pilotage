use std::time::{Duration, Instant};

use crate::score::{validate_gate_outcomes, validate_metric};
use crate::{
    AdapterError, CampaignBackend, CampaignMissionRuntime, GateEvaluator, GateOutcome,
    HardGateFailure, Journal, MetricEvaluator, SampleEvent, ScenarioRuntime, ScenarioStopReason,
    SearchStage, TelemetrySample, TuneError,
};
use pilotage_mission_core::{EngineState, MissionTerminal};

use super::contract::{RunContext, RunTerminal, adapter_error, evaluator_error, validate_sample};

pub(super) fn stream_samples<B, G, M>(
    journal: &Journal,
    stage: &SearchStage,
    context: &RunContext<'_>,
    backend: &mut B,
    mission: &mut CampaignMissionRuntime,
    gates: &mut G,
    metric: &mut M,
) -> RunTerminal
where
    B: CampaignBackend,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    let timeout = Duration::from_nanos(context.scenario.sample_timeout_ns);
    let wall_start = Instant::now();
    let mut position = StreamPosition::default();
    let outcome = stream_loop(
        journal,
        stage,
        context,
        backend,
        mission,
        gates,
        metric,
        timeout,
        wall_start,
        &mut position,
    );
    close_action_runtime(backend, mission, outcome)
}

#[allow(clippy::too_many_arguments)]
fn stream_loop<B, G, M>(
    journal: &Journal,
    stage: &SearchStage,
    context: &RunContext<'_>,
    backend: &mut B,
    mission: &mut CampaignMissionRuntime,
    gates: &mut G,
    metric: &mut M,
    timeout: Duration,
    wall_start: Instant,
    position: &mut StreamPosition,
) -> StreamOutcome
where
    B: CampaignBackend,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    loop {
        if let Err(error) = journal.ensure_usable() {
            return StreamOutcome::failed(error);
        }
        match backend.sample_blocking(timeout) {
            Ok(SampleEvent::Sample(sample)) => {
                if let Some(terminal) = validate_sample_before_action(context, &sample, position) {
                    return StreamOutcome::from_terminal(terminal);
                }
                if let Err(error) =
                    advance_mission(journal, backend, mission, &sample, elapsed_ns(wall_start))
                {
                    return StreamOutcome::failed(error);
                }
                if let Some(terminal) =
                    process_valid_sample(journal, stage, context, sample, position, gates, metric)
                {
                    return StreamOutcome::from_terminal(terminal);
                }
            }
            Ok(SampleEvent::Complete) => {
                if position.expected_sequence == 0 {
                    return StreamOutcome::from_terminal(finish_stream_blocking(
                        journal, context, 0, gates, metric,
                    ));
                }
                if let Err(error) = require_completed_mission(backend, mission) {
                    return StreamOutcome::failed(error);
                }
                let terminal = finish_stream_blocking(
                    journal,
                    context,
                    position.expected_sequence,
                    gates,
                    metric,
                );
                return StreamOutcome::completed(terminal, mission);
            }
            Ok(SampleEvent::TimedOut) => {
                return StreamOutcome {
                    terminal: hard_failure(
                        context,
                        position.expected_sequence,
                        position.elapsed_ms,
                        GateOutcome::fail(
                            "core.sample_timeout",
                            "the simulator did not supply a sample before the timeout",
                        ),
                    ),
                    reason: ScenarioStopReason::SampleTimeout,
                };
            }
            Err(source) => {
                return StreamOutcome::failed(adapter_error(backend, "sample", source));
            }
        }
    }
}

fn close_action_runtime<B: CampaignBackend>(
    backend: &mut B,
    mission: &mut CampaignMissionRuntime,
    outcome: StreamOutcome,
) -> RunTerminal {
    let adapter = backend.scenario_runtime().identity().id.clone();
    let closed = mission.stop_and_cleanup_blocking(
        backend.scenario_runtime_mut(),
        outcome.reason,
        mission.last_consumed_source_sequence(),
    );
    let Err(source) = closed else {
        return outcome.terminal;
    };
    let terminal = TuneError::Adapter {
        adapter,
        operation: "close scenario action runtime",
        source: AdapterError::new(source.to_string()),
    };
    match outcome.terminal {
        RunTerminal::Failed { error: primary } => RunTerminal::Failed {
            error: TuneError::OperationAndTerminalFailed {
                operation: "execute scenario action runtime",
                primary: Box::new(primary),
                terminal: Box::new(terminal),
            },
        },
        RunTerminal::Passed { .. } | RunTerminal::HardGate { .. } => {
            RunTerminal::Failed { error: terminal }
        }
    }
}

struct StreamOutcome {
    terminal: RunTerminal,
    reason: ScenarioStopReason,
}

impl StreamOutcome {
    fn failed(error: TuneError) -> Self {
        Self {
            terminal: RunTerminal::Failed { error },
            reason: ScenarioStopReason::ExecutionError,
        }
    }

    fn from_terminal(terminal: RunTerminal) -> Self {
        let reason = match &terminal {
            RunTerminal::HardGate { .. } => ScenarioStopReason::HardGate,
            RunTerminal::Failed { .. } => ScenarioStopReason::ExecutionError,
            RunTerminal::Passed { .. } => ScenarioStopReason::ExecutionError,
        };
        Self { terminal, reason }
    }

    fn completed(terminal: RunTerminal, mission: &CampaignMissionRuntime) -> Self {
        let reason = match (mission.state(), &terminal) {
            (Some(EngineState::Terminal { result }), RunTerminal::Passed { .. }) => {
                ScenarioStopReason::Mission(result)
            }
            (_, RunTerminal::HardGate { .. }) => ScenarioStopReason::HardGate,
            _ => ScenarioStopReason::ExecutionError,
        };
        Self { terminal, reason }
    }
}

fn advance_mission<B: CampaignBackend>(
    journal: &Journal,
    backend: &mut B,
    mission: &mut CampaignMissionRuntime,
    sample: &TelemetrySample,
    wall_time_ns: u64,
) -> Result<(), TuneError> {
    if matches!(mission.state(), Some(EngineState::Terminal { .. })) {
        return require_completed_mission(backend, mission);
    }
    let frame = backend
        .project_scenario_frame(sample)
        .map_err(|source| runtime_error(backend, "project scenario frame", source))?;
    mission
        .advance_authorized_blocking(
            backend.scenario_runtime_mut(),
            &frame,
            wall_time_ns,
            &mut || {
                journal
                    .ensure_usable()
                    .map_err(|source| crate::ScenarioRuntimeError::Authority { source })
            },
        )
        .map_err(|source| {
            runtime_error(
                backend,
                "advance mission engine",
                AdapterError::new(source.to_string()),
            )
        })?;
    require_valid_terminal(backend, mission)
}

fn elapsed_ns(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn require_valid_terminal<B: CampaignBackend>(
    backend: &B,
    mission: &CampaignMissionRuntime,
) -> Result<(), TuneError> {
    match mission.state() {
        Some(EngineState::Terminal {
            result: MissionTerminal::Complete { .. },
        })
        | Some(EngineState::Running { .. })
        | Some(EngineState::CleaningUp { .. }) => Ok(()),
        Some(EngineState::Terminal { result }) => Err(runtime_error(
            backend,
            "execute mission",
            AdapterError::new(format!("mission ended without completion: {result:?}")),
        )),
        None => Ok(()),
    }
}

fn require_completed_mission<B: CampaignBackend>(
    backend: &B,
    mission: &CampaignMissionRuntime,
) -> Result<(), TuneError> {
    if matches!(
        mission.state(),
        Some(EngineState::Terminal {
            result: MissionTerminal::Complete { .. }
        })
    ) {
        Ok(())
    } else {
        Err(runtime_error(
            backend,
            "complete scenario stream",
            AdapterError::new("the simulator completed before the mission engine"),
        ))
    }
}

fn runtime_error<B: CampaignBackend>(
    backend: &B,
    operation: &'static str,
    source: AdapterError,
) -> TuneError {
    TuneError::Adapter {
        adapter: backend.scenario_runtime().identity().id.clone(),
        operation,
        source,
    }
}

#[derive(Default)]
struct StreamPosition {
    expected_sequence: u64,
    elapsed_ms: u64,
}

fn validate_sample_before_action(
    context: &RunContext<'_>,
    sample: &TelemetrySample,
    position: &StreamPosition,
) -> Option<RunTerminal> {
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
    validate_sample(sample, position.expected_sequence, position.elapsed_ms)
        .err()
        .map(|error| RunTerminal::Failed { error })
}

#[allow(clippy::too_many_arguments)]
fn process_valid_sample<G, M>(
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
        Err(error) => Some(RunTerminal::Failed { error }),
    }
}

fn finish_stream_blocking<G, M>(
    journal: &Journal,
    context: &RunContext<'_>,
    sample_count: u64,
    gates: &mut G,
    metric: &mut M,
) -> RunTerminal
where
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
    finish_normal_run_blocking(journal, gates, metric)
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

fn finish_normal_run_blocking<G, M>(journal: &Journal, gates: &mut G, metric: &mut M) -> RunTerminal
where
    G: GateEvaluator,
    M: MetricEvaluator,
{
    if let Err(error) = journal.ensure_usable() {
        return RunTerminal::Failed { error };
    }
    if let Err(source) = gates.finish() {
        return RunTerminal::Failed {
            error: evaluator_error(gates.identity(), "finish hard gates", source),
        };
    }
    if let Err(error) = journal.ensure_usable() {
        return RunTerminal::Failed { error };
    }
    let values = match metric.finish() {
        Ok(values) => values,
        Err(source) => {
            return RunTerminal::Failed {
                error: evaluator_error(metric.identity(), "finish metric", source),
            };
        }
    };
    if let Err(error) = validate_metric(&values) {
        return RunTerminal::Failed { error };
    }
    RunTerminal::Passed { values }
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
            mission_revision_id: context.scenario.revision_id.clone(),
            repetition: context.repetition,
            seed: context.seed,
            sample_sequence,
            elapsed_ms,
            gate,
        },
    }
}
