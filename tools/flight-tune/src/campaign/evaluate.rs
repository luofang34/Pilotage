mod cleanup;
mod contract;
mod mission;
mod pending;
mod record;
mod stream;
mod terminal;

use crate::identity::digest_bytes;
use crate::journal::AttemptRole;
use crate::model::derive_seed;
use crate::{
    CampaignBackend, Candidate, CandidateTransitionReference, Digest, GateEvaluator, Journal,
    MetricEvaluator, RunExecutionContext, RunRecord, RunTerminalAdapter, SearchStage,
    SimulatorCapability, SimulatorVehicleAdapter, TuneError, VehicleBinding,
};
use contract::*;
use record::{RunProgress, record_committed_terminal};
use stream::stream_samples;
use terminal::{finish_live_terminal_blocking, prepare_live_terminal_blocking};

pub(super) use pending::{recover_pending_blocking, recover_pending_for_open_blocking};

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
    B: CampaignBackend,
    V: SimulatorVehicleAdapter + RunTerminalAdapter,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    journal.ensure_usable()?;
    let set = role.scenario_set();
    let scenario_count = scenarios(stage, set).len();
    let expected_runs = scenario_count * stage.repetitions as usize;
    let Some(cursor) = pending::resume_committed_prefix(
        journal,
        trial_id,
        role,
        candidate_digest,
        expected_runs,
        backend,
        gates,
        metric,
    )?
    else {
        return Ok(());
    };
    run_from_cursor_blocking(
        journal,
        stage,
        trial_id,
        role,
        candidate,
        candidate_digest,
        transition,
        expected_runs,
        cursor,
        backend,
        vehicle,
        capability,
        gates,
        metric,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_from_cursor_blocking<B, V, G, M>(
    journal: &mut Journal,
    stage: &SearchStage,
    trial_id: u64,
    role: AttemptRole,
    candidate: &Candidate,
    candidate_digest: Digest,
    transition: Option<CandidateTransitionReference>,
    expected_runs: usize,
    mut cursor: pending::ResumeCursor,
    backend: &mut B,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    gates: &mut G,
    metric: &mut M,
) -> Result<(), TuneError>
where
    B: CampaignBackend,
    V: SimulatorVehicleAdapter + RunTerminalAdapter,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    let set = role.scenario_set();
    let mut run_index = 0_u64;
    for scenario in scenarios(stage, set) {
        for repetition in 0..stage.repetitions {
            if run_index < cursor.next_run_index {
                run_index = run_index.wrapping_add(1);
                continue;
            }
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
            let progress = run_one_blocking(
                journal,
                stage,
                &context,
                trial_id,
                role,
                candidate,
                candidate_digest,
                expected_runs,
                &mut cursor.runs,
                backend,
                vehicle,
                capability,
                gates,
                metric,
            )?;
            if matches!(progress, RunProgress::Complete) {
                return Ok(());
            }
        }
    }
    Err(no_run_result())
}

fn no_run_result() -> TuneError {
    TuneError::InvalidState {
        operation: "evaluate candidate",
        detail: "the run plan produced no result".to_owned(),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_one_blocking<B, V, G, M>(
    journal: &mut Journal,
    stage: &SearchStage,
    context: &RunContext<'_>,
    trial_id: u64,
    role: AttemptRole,
    candidate: &Candidate,
    candidate_digest: Digest,
    expected_runs: usize,
    runs: &mut Vec<RunRecord>,
    backend: &mut B,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    gates: &mut G,
    metric: &mut M,
) -> Result<RunProgress, TuneError>
where
    B: CampaignBackend,
    V: SimulatorVehicleAdapter + RunTerminalAdapter,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    let mut prepared = prepare_live_terminal_blocking(journal, context, vehicle, capability)?;
    let semantic = match prepared.binding_error.take() {
        Some(error) => RunTerminal::Failed { error },
        None => run_until_terminal(
            journal,
            stage,
            context,
            candidate,
            candidate_digest,
            backend,
            vehicle,
            capability,
            gates,
            metric,
        ),
    };
    let committed = finish_live_terminal_blocking(
        journal, stage, context, semantic, &prepared, backend, vehicle, capability,
    )?;
    record_committed_terminal(
        journal,
        trial_id,
        role,
        expected_runs,
        runs,
        &committed.receipt,
        committed.primary_error,
        backend,
        gates,
        metric,
    )
}

#[allow(clippy::too_many_arguments)]
fn prepare_run_context<'a>(
    journal: &mut Journal,
    trial_id: u64,
    role: AttemptRole,
    candidate_digest: Digest,
    transition: Option<CandidateTransitionReference>,
    set: crate::ScenarioSet,
    scenario: &'a crate::MissionReference,
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
        run_index,
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
    B: CampaignBackend,
    V: SimulatorVehicleAdapter,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    let mut mission = match mission::admit_campaign_mission(journal, context, backend) {
        Ok(mission) => mission,
        Err(error) => return RunTerminal::Failed { error },
    };
    if let Err(error) = journal.ensure_usable() {
        return RunTerminal::Failed { error };
    }
    if let Err(error) = prepare_backend_run(backend, capability, context) {
        return RunTerminal::Failed { error };
    }
    if let Err(error) = ensure_candidate_for_run_blocking(
        journal,
        vehicle,
        capability,
        context,
        candidate,
        candidate_digest,
    ) {
        return RunTerminal::Failed { error };
    }
    if let Err(error) = begin_evaluators(journal, context.scenario, gates, metric) {
        return RunTerminal::Failed { error };
    }
    if let Err(error) = journal.ensure_usable() {
        return RunTerminal::Failed { error };
    }
    let receipt = match backend.start_blocking(capability, &context.execution) {
        Ok(receipt) => receipt,
        Err(source) => {
            return RunTerminal::Failed {
                error: adapter_error(backend, "start", source),
            };
        }
    };
    if let Err(error) = validate_scenario_receipt(receipt, capability, context) {
        return RunTerminal::Failed { error };
    }
    if let Err(error) = mission::start_campaign_action_port(journal, context, backend, &mut mission)
    {
        return RunTerminal::Failed { error };
    }
    stream_samples(
        journal,
        stage,
        context,
        backend,
        &mut mission,
        gates,
        metric,
    )
}

fn prepare_backend_run<B>(
    backend: &mut B,
    capability: &SimulatorCapability,
    context: &RunContext<'_>,
) -> Result<(), TuneError>
where
    B: CampaignBackend,
{
    let receipt = backend
        .prepare_blocking(capability, &context.execution, context.scenario)
        .map_err(|source| adapter_error(backend, "prepare", source))?;
    validate_run_preparation_receipt(receipt, capability, context.run_intent_digest)
}
