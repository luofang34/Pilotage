use crate::journal::replay::{JournalState, PendingAttempt, invalid};
use crate::journal::{JournalEvent, SessionIdentity};
use crate::model::derive_seed;
use crate::{
    CandidateEvaluation, Digest, RunExecutionContext, ScenarioRef, ScenarioSet, SearchStage,
    TuneError,
};

#[derive(Debug, Clone)]
pub(crate) struct PreparedRun {
    pub(super) run_index: u64,
    pub(super) context: RunExecutionContext,
    pub(super) run_intent_digest: Digest,
}

impl JournalState {
    pub(crate) fn pending_role(&self, trial_id: u64) -> Result<crate::AttemptRole, TuneError> {
        self.pending
            .as_ref()
            .filter(|pending| pending.trial_id == trial_id && pending.outcome.is_none())
            .map(|pending| pending.role)
            .ok_or_else(|| invalid("the trial is not pending or already has an outcome"))
    }
}

pub(super) fn prepare_event(
    state: &mut JournalState,
    event: &JournalEvent,
    stage: &SearchStage,
    session: &SessionIdentity,
) -> Result<(), TuneError> {
    let JournalEvent::RunPrepared {
        trial_id,
        run_index,
        context,
        run_intent_digest,
    } = event
    else {
        return Err(invalid("the event is not a run preparation"));
    };
    prepare(
        state,
        *trial_id,
        *run_index,
        context,
        *run_intent_digest,
        stage,
        session,
    )
}

pub(super) fn prepare(
    state: &mut JournalState,
    trial_id: u64,
    run_index: u64,
    context: &RunExecutionContext,
    run_intent_digest: Digest,
    stage: &SearchStage,
    session: &SessionIdentity,
) -> Result<(), TuneError> {
    context
        .validate()
        .map_err(|_| invalid("a prepared run context is not valid during replay"))?;
    let context_digest = context
        .digest()
        .map_err(|_| invalid("a prepared run context digest cannot be recomputed"))?;
    let pending = state
        .pending
        .as_mut()
        .filter(|pending| pending.outcome.is_none())
        .ok_or_else(|| invalid("run preparation has no active attempt"))?;
    let expected_index = u64::try_from(pending.prepared_runs.len())
        .map_err(|_| invalid("prepared run index overflow"))?;
    let session_digest = super::super::storage::document_digest("session identity", session)?;
    let expected_scenario = expected_scenario(stage, pending.role.scenario_set(), run_index)?;
    let expected_seed = derive_seed(
        session.fixed_seed,
        pending.role.scenario_set(),
        expected_scenario.0,
        expected_scenario.1,
    );
    if trial_id != pending.trial_id
        || run_index != expected_index
        || context.tuning_session_digest() != session_digest
        || context.trial_id() != pending.trial_id
        || context.role() != pending.role
        || context.candidate_digest() != pending.candidate
        || context.transition_authorization() != pending.transition
        || context.scenario_set() != pending.role.scenario_set()
        || context.scenario_id() != expected_scenario.0.id
        || context.scenario_digest() != expected_scenario.0.digest
        || context.repetition() != expected_scenario.1
        || context.seed() != expected_seed
        || run_intent_digest.is_zero()
        || context_digest != run_intent_digest
    {
        return Err(invalid("a prepared run does not match its attempt plan"));
    }
    pending.prepared_runs.push(PreparedRun {
        run_index,
        context: context.clone(),
        run_intent_digest,
    });
    Ok(())
}

pub(super) fn validate_outcome(
    pending: &PendingAttempt,
    evaluation: &CandidateEvaluation,
) -> Result<(), TuneError> {
    let prepared_identity_is_exact =
        pending
            .prepared_runs
            .iter()
            .enumerate()
            .all(|(index, run)| {
                u64::try_from(index) == Ok(run.run_index)
                    && run
                        .context
                        .digest()
                        .is_ok_and(|digest| digest == run.run_intent_digest)
            });
    let prepared_count = pending.prepared_runs.len();
    let count_matches = match evaluation {
        CandidateEvaluation::Passed { runs, .. } => prepared_count == runs.len(),
        CandidateEvaluation::HardGateFailed { completed_runs, .. } => {
            prepared_count == completed_runs.len().saturating_add(1)
        }
        CandidateEvaluation::Quarantined { .. } => true,
    };
    if count_matches && prepared_identity_is_exact {
        Ok(())
    } else {
        Err(invalid(
            "an attempt outcome does not match its prepared run count",
        ))
    }
}

fn expected_scenario(
    stage: &SearchStage,
    set: ScenarioSet,
    run_index: u64,
) -> Result<(&ScenarioRef, u32), TuneError> {
    let repetitions = u64::from(stage.repetitions);
    let scenario_index = usize::try_from(run_index / repetitions)
        .map_err(|_| invalid("prepared run scenario index overflow"))?;
    let repetition = u32::try_from(run_index % repetitions)
        .map_err(|_| invalid("prepared run repetition overflow"))?;
    let scenarios = match set {
        ScenarioSet::Training => &stage.training_scenarios,
        ScenarioSet::Promotion => &stage.promotion_scenarios,
        ScenarioSet::FinalQualification => &stage.final_qualification_scenarios,
    };
    scenarios
        .get(scenario_index)
        .map(|scenario| (scenario, repetition))
        .ok_or_else(|| invalid("prepared run exceeds the attempt plan"))
}
