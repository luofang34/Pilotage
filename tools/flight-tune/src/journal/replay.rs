use crate::journal::{
    AttemptRole, AuthenticatedEvaluationProof, CampaignPhase, FinalQualificationOutcome,
    JOURNAL_SCHEMA_VERSION, JournalEntry, JournalEvent, PromotionClosure, PromotionDecision,
    SessionIdentity,
};
use crate::{
    CandidateEvaluation, CandidateTransitionReference, Digest, SearchStage, TrainingObservation,
    TuneError,
};

mod attempt;
pub(crate) mod plan;
mod run;
pub(super) mod terminal;
pub(crate) mod transition;

use super::transition::AuthorizedTrainingTransition;
pub(super) use run::{PreparedRun, PreparedRunTerminalState};

#[derive(Debug, Clone)]
pub(crate) struct PendingAttempt {
    pub(crate) trial_id: u64,
    pub(crate) role: AttemptRole,
    pub(crate) candidate: Digest,
    pub(crate) plan_digest: Digest,
    pub(crate) transition: Option<CandidateTransitionReference>,
    pub(crate) prepared_runs: Vec<PreparedRun>,
    pub(crate) outcome: Option<PendingOutcome>,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingOutcome {
    pub(crate) evaluation: CandidateEvaluation,
    pub(crate) selected: Option<bool>,
    pub(crate) proof: Option<AuthenticatedEvaluationProof>,
}

#[derive(Debug, Clone)]
pub(crate) struct JournalState {
    pub(crate) phase: CampaignPhase,
    pub(crate) training_incumbent: Digest,
    pub(crate) training_incumbent_evaluation: Option<CandidateEvaluation>,
    pub(crate) training_baseline: Option<CandidateEvaluation>,
    pub(crate) training_attempt_count: u64,
    pub(crate) training_history: Vec<TrainingObservation>,
    pub(crate) next_trial_id: u64,
    pub(crate) authorized_transition: Option<AuthorizedTrainingTransition>,
    pub(crate) pending: Option<PendingAttempt>,
    pub(crate) frozen_candidate: Option<Digest>,
    pub(crate) promotion_baseline: Option<CandidateEvaluation>,
    pub(crate) promotion_baseline_proof: Option<AuthenticatedEvaluationProof>,
    pub(crate) promotion_frozen: Option<CandidateEvaluation>,
    pub(crate) promotion_frozen_proof: Option<AuthenticatedEvaluationProof>,
    pub(crate) promotion_decision: Option<PromotionDecision>,
    pub(crate) promotion_closure: Option<PromotionClosure>,
    pub(crate) final_evaluation: Option<CandidateEvaluation>,
    pub(crate) final_proof: Option<AuthenticatedEvaluationProof>,
    pub(crate) final_outcome: Option<FinalQualificationOutcome>,
}

impl JournalState {
    pub(crate) fn settlement_candidate(&self, initial: Digest) -> Digest {
        if self
            .promotion_decision
            .as_ref()
            .is_some_and(PromotionDecision::is_promoted)
        {
            self.frozen_candidate.unwrap_or(initial)
        } else {
            initial
        }
    }

    pub(crate) fn authorized_final_candidate(&self, initial: Digest) -> Result<Digest, TuneError> {
        let closure = self
            .promotion_closure
            .as_ref()
            .ok_or_else(|| invalid("final qualification has no promotion closure"))?;
        match closure.decision {
            PromotionDecision::Promoted { .. } => closure
                .selected_candidate
                .filter(|candidate| Some(*candidate) == self.frozen_candidate)
                .ok_or_else(|| invalid("promotion did not authorize the frozen candidate")),
            PromotionDecision::RejectedNoImprovement { .. } => closure
                .selected_candidate
                .filter(|candidate| *candidate == initial)
                .ok_or_else(|| invalid("promotion did not authorize the initial candidate")),
            PromotionDecision::RejectedHardGate { .. }
            | PromotionDecision::Indeterminate { .. } => Err(invalid(
                "the promotion result does not authorize final qualification",
            )),
        }
    }

    pub(crate) fn authenticated_proof(
        &self,
        trial_id: u64,
        evaluation: &CandidateEvaluation,
    ) -> Result<Option<AuthenticatedEvaluationProof>, TuneError> {
        let pending = self
            .pending
            .as_ref()
            .filter(|pending| pending.trial_id == trial_id && pending.outcome.is_none())
            .ok_or_else(|| invalid("the attempt is not pending or already has an outcome"))?;
        if matches!(
            pending.role,
            AttemptRole::TrainingBaseline | AttemptRole::TrainingChallenger { .. }
        ) {
            return Ok(None);
        }
        Ok(Some(AuthenticatedEvaluationProof::new(
            pending.trial_id,
            pending.role,
            pending.candidate,
            pending.plan_digest,
            evaluation.clone(),
            terminal::owned_committed_receipts(pending)?,
        )?))
    }
}

pub(super) fn replay(
    entries: &[JournalEntry],
    entry_digests: &[Digest],
    stage: &SearchStage,
) -> Result<JournalState, TuneError> {
    let Some(first) = entries.first() else {
        return Err(invalid("the journal is empty"));
    };
    if entries.len() != entry_digests.len() {
        return Err(invalid("the journal digest count does not match"));
    }
    validate_header(first, 0, None, &first.session)?;
    let JournalEvent::Started { candidate } = first.event else {
        return Err(invalid("the first event is not a start event"));
    };
    if candidate != first.session.initial_candidate_digest {
        return Err(invalid("the initial candidate digest does not match"));
    }
    let mut state = initial_state(candidate);
    for (index, entry) in entries.iter().enumerate().skip(1) {
        let sequence = u64::try_from(index).map_err(|_| invalid("sequence overflow"))?;
        validate_header(
            entry,
            sequence,
            Some(entry_digests[index - 1]),
            &first.session,
        )?;
        apply_event(&mut state, &entry.event, stage, &first.session)?;
    }
    Ok(state)
}

fn initial_state(candidate: Digest) -> JournalState {
    JournalState {
        phase: CampaignPhase::Searching,
        training_incumbent: candidate,
        training_incumbent_evaluation: None,
        training_baseline: None,
        training_attempt_count: 0,
        training_history: Vec::new(),
        next_trial_id: 0,
        authorized_transition: None,
        pending: None,
        frozen_candidate: None,
        promotion_baseline: None,
        promotion_baseline_proof: None,
        promotion_frozen: None,
        promotion_frozen_proof: None,
        promotion_decision: None,
        promotion_closure: None,
        final_evaluation: None,
        final_proof: None,
        final_outcome: None,
    }
}

fn validate_header(
    entry: &JournalEntry,
    sequence: u64,
    previous: Option<Digest>,
    session: &crate::SessionIdentity,
) -> Result<(), TuneError> {
    if entry.schema_version != JOURNAL_SCHEMA_VERSION
        || entry.sequence != sequence
        || entry.previous != previous
        || &entry.session != session
    {
        return Err(invalid(format!(
            "journal entry {} has the wrong header",
            entry.sequence
        )));
    }
    Ok(())
}

fn apply_event(
    state: &mut JournalState,
    event: &JournalEvent,
    stage: &SearchStage,
    session: &SessionIdentity,
) -> Result<(), TuneError> {
    let initial = session.initial_candidate_digest;
    let fixed_seed = session.fixed_seed;
    if state.authorized_transition.is_some()
        && !matches!(event, JournalEvent::AttemptPrepared { .. })
    {
        return Err(invalid(
            "a transition authorization must be followed by its attempt",
        ));
    }
    match event {
        JournalEvent::Started { .. } => Err(invalid("the journal has two start events")),
        event @ JournalEvent::CandidateTransitionAuthorized { .. } => {
            transition::authorize_event(state, event, stage, session)
        }
        JournalEvent::AttemptPrepared {
            trial_id,
            role,
            candidate,
            plan_digest,
            transition,
        } => attempt::prepare(
            state,
            *trial_id,
            *role,
            *candidate,
            *plan_digest,
            transition.as_ref(),
            stage,
            initial,
            fixed_seed,
        ),
        event @ JournalEvent::RunPrepared { .. } => {
            run::prepare_event(state, event, stage, session)
        }
        event @ (JournalEvent::RunBound { .. }
        | JournalEvent::RunTerminalIntentPrepared { .. }
        | JournalEvent::RunTerminalReportRecorded { .. }
        | JournalEvent::RunTerminalEvidenceFailureRecorded { .. }
        | JournalEvent::RunCommitted { .. }) => terminal::apply_event(state, event, session),
        event @ JournalEvent::AttemptCompleted { .. } => {
            apply_attempt_completion(state, event, stage, fixed_seed)
        }
        JournalEvent::AttemptQuarantined {
            trial_id,
            reason,
            proof,
        } => attempt::quarantine(state, *trial_id, reason, proof.as_deref()),
        JournalEvent::CleanupRecorded { trial_id, cleanup } => {
            attempt::cleanup(state, *trial_id, cleanup)
        }
        JournalEvent::Frozen {
            baseline,
            candidate,
        } => close::freeze(state, *baseline, *candidate, initial),
        JournalEvent::PromotionClosed { closure } => {
            close::promotion(state, closure, stage, session)
        }
        event @ JournalEvent::Sealed { .. } => apply_seal(state, event, initial, stage),
    }
}

fn apply_attempt_completion(
    state: &mut JournalState,
    event: &JournalEvent,
    stage: &SearchStage,
    fixed_seed: u64,
) -> Result<(), TuneError> {
    let JournalEvent::AttemptCompleted {
        trial_id,
        evaluation,
        proof,
        selected_as_training_incumbent,
    } = event
    else {
        return Err(invalid("the event is not an attempt completion"));
    };
    attempt::complete(
        state,
        *trial_id,
        evaluation,
        proof.as_deref(),
        *selected_as_training_incumbent,
        stage,
        fixed_seed,
    )
}

fn apply_seal(
    state: &mut JournalState,
    event: &JournalEvent,
    initial: Digest,
    stage: &SearchStage,
) -> Result<(), TuneError> {
    let JournalEvent::Sealed {
        candidate,
        outcome,
        promotion_closure_digest,
        final_evaluation_digest,
        final_proof_digest,
    } = event
    else {
        return Err(invalid("the event is not a final seal"));
    };
    close::seal(
        state,
        *candidate,
        outcome,
        *promotion_closure_digest,
        *final_evaluation_digest,
        *final_proof_digest,
        initial,
        stage,
    )
}

fn invalid(detail: impl Into<String>) -> TuneError {
    TuneError::InvalidJournal {
        detail: detail.into(),
    }
}
mod close;

pub(crate) use close::{expected_promotion_closure, expected_promotion_closure_from_proofs};
