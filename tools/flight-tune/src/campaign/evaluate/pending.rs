use crate::journal::AttemptRole;
use crate::journal::snapshot::RunTerminalSnapshot;
use crate::{
    CampaignBackend, CandidateTransitionReference, Digest, GateEvaluator, Journal, MetricEvaluator,
    RunTerminalAdapter, SearchStage, SimulatorCapability, SimulatorVehicleAdapter, TuneError,
    VehicleBinding,
};

use super::cleanup::finish_cleanup;
use super::record::{RunProgress, completed_scenario_run, record_committed_terminal};
use super::terminal::recover_current_run_blocking;

pub(super) struct ResumeCursor {
    pub(super) runs: Vec<crate::RunRecord>,
    pub(super) next_run_index: u64,
}

struct PendingResume {
    trial_id: u64,
    role: AttemptRole,
    candidate: Digest,
    transition: Option<CandidateTransitionReference>,
    expected_runs: usize,
    cursor: ResumeCursor,
}

#[allow(clippy::too_many_arguments)]
pub(in crate::campaign) fn recover_pending_blocking<B, V, G, M>(
    journal: &mut Journal,
    stage: &SearchStage,
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
    let resume = recover_terminal_prefix_blocking(
        journal, stage, backend, vehicle, capability, gates, metric,
    )?;
    resume_pending_blocking(
        journal, stage, resume, backend, vehicle, capability, gates, metric,
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::campaign) fn recover_pending_for_open_blocking<B, V, G, M>(
    journal: &mut Journal,
    stage: &SearchStage,
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
    let resume = recover_terminal_prefix_blocking(
        journal, stage, backend, vehicle, capability, gates, metric,
    )?;
    super::super::transition::reauthorize_saved(journal, stage, vehicle)?;
    resume_pending_blocking(
        journal, stage, resume, backend, vehicle, capability, gates, metric,
    )
}

#[allow(clippy::too_many_arguments)]
fn recover_terminal_prefix_blocking<B, V, G, M>(
    journal: &mut Journal,
    stage: &SearchStage,
    backend: &mut B,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    gates: &mut G,
    metric: &mut M,
) -> Result<Option<PendingResume>, TuneError>
where
    B: CampaignBackend,
    V: SimulatorVehicleAdapter + RunTerminalAdapter,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    journal.ensure_usable()?;
    let Some(snapshot) = journal.pending_attempt_snapshot() else {
        return Ok(None);
    };
    if snapshot.outcome.is_some() {
        // A session that stopped between a quarantine and its decision left
        // the decision owed. Cleanup cannot close the attempt until the
        // declared limit has answered it.
        if let Some(trial_id) = journal.awaits_retry_decision() {
            journal.record_retry_decision(trial_id)?;
        }
        finish_cleanup(journal, snapshot.trial_id, backend, gates, metric, true)?;
        return Ok(None);
    }
    if let Some(run) = snapshot.current_run()
        && !matches!(run.terminal, RunTerminalSnapshot::Committed { .. })
    {
        recover_current_run_blocking(journal, run, backend, vehicle, capability)?;
    }
    pending_resume_blocking(journal, stage, backend, gates, metric)
}

#[allow(clippy::too_many_arguments)]
fn pending_resume_blocking<B, G, M>(
    journal: &mut Journal,
    stage: &SearchStage,
    backend: &mut B,
    gates: &mut G,
    metric: &mut M,
) -> Result<Option<PendingResume>, TuneError>
where
    B: CampaignBackend,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    let snapshot = journal
        .pending_attempt_snapshot()
        .ok_or_else(|| invalid_pending("the pending attempt snapshot is missing"))?;
    let expected_runs = crate::model::AttemptRunPlan::new(stage, snapshot.role)?.run_count();
    let cursor = resume_committed_prefix(
        journal,
        snapshot.trial_id,
        snapshot.role,
        snapshot.candidate,
        expected_runs,
        backend,
        gates,
        metric,
    )
    .map_err(recovery_error)?;
    Ok(cursor.map(|cursor| PendingResume {
        trial_id: snapshot.trial_id,
        role: snapshot.role,
        candidate: snapshot.candidate,
        transition: snapshot.transition,
        expected_runs,
        cursor,
    }))
}

#[allow(clippy::too_many_arguments)]
fn resume_pending_blocking<B, V, G, M>(
    journal: &mut Journal,
    stage: &SearchStage,
    resume: Option<PendingResume>,
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
    let Some(resume) = resume else {
        return Ok(());
    };
    let candidate = journal.read_candidate(resume.candidate)?;
    super::run_from_cursor_blocking(
        journal,
        stage,
        resume.trial_id,
        resume.role,
        &candidate,
        resume.candidate,
        resume.transition,
        resume.expected_runs,
        resume.cursor,
        backend,
        vehicle,
        capability,
        gates,
        metric,
    )
    .map_err(recovery_error)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn resume_committed_prefix<B, G, M>(
    journal: &mut Journal,
    trial_id: u64,
    role: AttemptRole,
    candidate_digest: Digest,
    expected_runs: usize,
    backend: &mut B,
    gates: &mut G,
    metric: &mut M,
) -> Result<Option<ResumeCursor>, TuneError>
where
    B: CampaignBackend,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    let snapshot = journal
        .pending_attempt_snapshot()
        .ok_or_else(|| invalid_pending("the pending attempt snapshot is missing"))?;
    validate_pending(&snapshot, trial_id, role, candidate_digest)?;
    let receipts = snapshot.committed_prefix();
    if receipts.len() != snapshot.prepared_runs.len() {
        return Err(invalid_pending(
            "terminal recovery did not commit the current prepared run",
        ));
    }
    let Some((last, prefix)) = receipts.split_last() else {
        return Ok(Some(ResumeCursor {
            runs: Vec::new(),
            next_run_index: 0,
        }));
    };
    let mut runs = prefix
        .iter()
        .map(|receipt| completed_scenario_run(receipt))
        .collect::<Result<Vec<_>, _>>()?;
    let progress = record_committed_terminal(
        journal,
        trial_id,
        role,
        expected_runs,
        &mut runs,
        last,
        None,
        backend,
        gates,
        metric,
    )?;
    match progress {
        RunProgress::Complete => Ok(None),
        RunProgress::Continue => Ok(Some(ResumeCursor {
            runs,
            next_run_index: u64::try_from(receipts.len())
                .map_err(|_| invalid_pending("the committed run count is too large"))?,
        })),
    }
}

fn validate_pending(
    snapshot: &crate::journal::snapshot::PendingAttemptSnapshot,
    trial_id: u64,
    role: AttemptRole,
    candidate_digest: Digest,
) -> Result<(), TuneError> {
    if snapshot.trial_id != trial_id
        || snapshot.role != role
        || snapshot.candidate != candidate_digest
        || snapshot.outcome.is_some()
    {
        return Err(invalid_pending(
            "the pending attempt does not match the requested evaluation",
        ));
    }
    Ok(())
}

fn invalid_pending(detail: &str) -> TuneError {
    TuneError::InvalidState {
        operation: "recover pending attempt",
        detail: detail.to_owned(),
    }
}

fn recovery_error(error: TuneError) -> TuneError {
    if matches!(
        &error,
        TuneError::InvalidState {
            operation: "cleanup",
            ..
        }
    ) {
        return invalid_pending("cleanup did not restore an idle simulator");
    }
    error
}
