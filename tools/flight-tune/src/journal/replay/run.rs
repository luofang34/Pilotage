use crate::journal::replay::{JournalState, invalid};
use crate::journal::{JournalEvent, SessionIdentity};
use crate::model::derive_seed;
use crate::{
    Digest, RunBindingReceipt, RunExecutionContext, RunTerminalClass, RunTerminalCompletion,
    RunTerminalDisposition, RunTerminalIntent, RunTerminalPlan, RunTerminalReceipt,
    RunTerminalReport, SearchStage, TuneError,
};

#[derive(Debug, Clone)]
pub(crate) struct PreparedRun {
    pub(crate) run_index: u64,
    pub(crate) context: RunExecutionContext,
    pub(crate) run_intent_digest: Digest,
    pub(crate) terminal: PreparedRunTerminalState,
}

#[derive(Debug, Clone)]
pub(crate) enum PreparedRunTerminalState {
    Prepared,
    Bound {
        plan: RunTerminalPlan,
        binding: RunBindingReceipt,
    },
    IntentPrepared {
        plan: RunTerminalPlan,
        binding: RunBindingReceipt,
        intent: RunTerminalIntent,
    },
    ReportRecorded {
        binding: RunBindingReceipt,
        report: RunTerminalReport,
        base_class: RunTerminalClass,
        expected_receipt: Box<RunTerminalReceipt>,
    },
    EvidenceFailureRecorded {
        binding: RunBindingReceipt,
        report: RunTerminalReport,
        base_class: RunTerminalClass,
        expected_receipt: Box<RunTerminalReceipt>,
        class: RunTerminalClass,
    },
    Committed {
        receipt: Box<RunTerminalReceipt>,
    },
}

impl PreparedRunTerminalState {
    fn permits_next_run(&self) -> bool {
        let Self::Committed { receipt } = self else {
            return false;
        };
        matches!(
            receipt.class().disposition(),
            RunTerminalDisposition::Completed {
                completion: RunTerminalCompletion::ScenarioComplete
            }
        )
    }
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
    if pending
        .prepared_runs
        .last()
        .is_some_and(|run| !run.terminal.permits_next_run())
    {
        return Err(invalid(
            "a new run requires one committed completed scenario",
        ));
    }
    let session_digest = super::super::storage::document_digest("session identity", session)?;
    let plan = crate::model::AttemptRunPlan::new(stage, pending.role)?;
    let index = usize::try_from(run_index).map_err(|_| invalid("prepared run index overflow"))?;
    let expected_scenario = plan.run_at(index)?;
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
        || context.mission_revision_id() != expected_scenario.0.revision_id
        || context.mission_content_digest() != expected_scenario.0.content_digest
        || context.repetition() != expected_scenario.1
        || context.seed() != expected_seed
        || context.retry_index() != pending.retry_index
        || run_intent_digest.is_zero()
        || context_digest != run_intent_digest
    {
        return Err(invalid("a prepared run does not match its attempt plan"));
    }
    pending.prepared_runs.push(PreparedRun {
        run_index,
        context: context.clone(),
        run_intent_digest,
        terminal: PreparedRunTerminalState::Prepared,
    });
    Ok(())
}

