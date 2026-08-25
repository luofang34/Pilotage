mod cleanup;
mod contract;
mod record;
mod stream;

use crate::identity::digest_bytes;
use crate::journal::AttemptRole;
use crate::model::derive_seed;
use crate::{
    Candidate, CandidateTransitionReference, Digest, GateEvaluator, Journal, MetricEvaluator,
    RunExecutionContext, SearchStage, SimulatorBackend, SimulatorCapability,
    SimulatorVehicleAdapter, TuneError, VehicleBinding,
};
use cleanup::{cleanup_status, finish_cleanup, operation_status, quarantine_after_error};
use contract::*;
use record::{RunProgress, record_terminal};
use stream::stream_samples;

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

pub(super) fn ensure_settled_candidate_blocking<V>(
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
    let receipt = vehicle.adapter_mut().ensure_settled_candidate_blocking(
        capability,
        candidate,
        candidate_digest,
    );
    validate_candidate_receipt(receipt, capability, candidate_digest, None)?;
    journal.ensure_usable()
}

fn ensure_candidate_for_run_blocking<V>(
    journal: &Journal,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    context: &RunContext<'_>,
    candidate: &Candidate,
    candidate_digest: Digest,
) -> Result<(), TuneError>
where
    V: SimulatorVehicleAdapter,
{
    journal.ensure_usable()?;
    let receipt = vehicle.adapter_mut().ensure_candidate_for_run_blocking(
        capability,
        &context.execution,
        candidate,
        candidate_digest,
    );
    validate_candidate_receipt(
        receipt,
        capability,
        candidate_digest,
        Some(context.run_intent_digest),
    )?;
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
    transition: Option<CandidateTransitionReference>,
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
    let mut run_index = 0_u64;
    for scenario in scenarios(stage, set) {
        for repetition in 0..stage.repetitions {
            let context = prepare_run_context(
                journal,
                trial_id,
                role,
                candidate_digest,
                transition,
                set,
                scenario,
                repetition,
                run_index,
            )?;
            run_index = run_index.wrapping_add(1);
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
fn prepare_run_context<'a>(
    journal: &mut Journal,
    trial_id: u64,
    role: AttemptRole,
    candidate_digest: Digest,
    transition: Option<CandidateTransitionReference>,
    set: crate::ScenarioSet,
    scenario: &'a crate::ScenarioRef,
    repetition: u32,
    run_index: u64,
) -> Result<RunContext<'a>, TuneError> {
    journal.ensure_usable()?;
    let seed = derive_seed(journal.session().fixed_seed, set, scenario, repetition);
    let execution = RunExecutionContext::new(
        journal.session_digest()?,
        trial_id,
        role,
        candidate_digest,
        transition,
        set,
        scenario,
        repetition,
        seed,
    )?;
    let run_intent_digest = journal.prepare_run(run_index, &execution)?;
    Ok(RunContext {
        execution,
        run_intent_digest,
        set,
        scenario,
        repetition,
        seed,
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
    if let Err(error) = prepare_backend_run(backend, capability, context) {
        return RunTerminal::Failed {
            error,
            started: false,
        };
    }
    if let Err(error) = ensure_candidate_for_run_blocking(
        journal,
        vehicle,
        capability,
        context,
        candidate,
        candidate_digest,
    ) {
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
    let receipt = match backend.start_blocking(capability, &context.execution) {
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

fn prepare_backend_run<B>(
    backend: &mut B,
    capability: &SimulatorCapability,
    context: &RunContext<'_>,
) -> Result<(), TuneError>
where
    B: SimulatorBackend,
{
    let receipt = backend
        .prepare_blocking(capability, &context.execution, context.scenario)
        .map_err(|source| adapter_error(backend, "prepare", source))?;
    validate_run_preparation_receipt(receipt, capability, context.run_intent_digest)
}
