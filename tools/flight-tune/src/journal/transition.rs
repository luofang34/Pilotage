use crate::journal::{AttemptRole, SessionIdentity};
use crate::{
    Candidate, CandidateTransitionReceipt, CandidateTransitionReference, Digest,
    RunExecutionContext, SearchStage, TuneError,
};

use super::{Journal, JournalEvent, storage};

#[derive(Debug, Clone)]
pub(crate) struct AuthorizedTrainingTransition {
    pub(crate) attempt_index: u64,
    pub(crate) candidate: Digest,
    pub(crate) reference: CandidateTransitionReference,
}

impl Journal {
    pub(crate) fn authorized_training_transition(&self) -> Option<&AuthorizedTrainingTransition> {
        self.state.authorized_transition.as_ref()
    }

    pub(crate) fn authorize_training_transition(
        &mut self,
        attempt_index: u64,
        reason: impl Into<String>,
        candidate: &Candidate,
        receipt: CandidateTransitionReceipt,
    ) -> Result<CandidateTransitionReference, TuneError> {
        self.ensure_usable()?;
        candidate.validate()?;
        let target = self.record_storage_result(storage::store_candidate(
            &self.storage,
            &self.writer,
            candidate,
        ))?;
        let source_candidate = self.read_candidate(self.state.training_incumbent)?;
        let reference = validate_authorization(
            self.session(),
            &self.stage,
            attempt_index,
            &source_candidate,
            self.state.training_incumbent,
            candidate,
            target,
            &receipt,
        )?;
        self.append(JournalEvent::CandidateTransitionAuthorized {
            attempt_index,
            reason: reason.into(),
            candidate: target,
            receipt,
        })?;
        Ok(reference)
    }

    pub(crate) fn prepare_run(
        &mut self,
        run_index: u64,
        context: &RunExecutionContext,
    ) -> Result<Digest, TuneError> {
        self.ensure_usable()?;
        let run_intent_digest = context.digest()?;
        self.append(JournalEvent::RunPrepared {
            trial_id: context.trial_id(),
            run_index,
            context: context.clone(),
            run_intent_digest,
        })?;
        Ok(run_intent_digest)
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn validate_authorization(
    session: &SessionIdentity,
    stage: &SearchStage,
    attempt_index: u64,
    source: &Candidate,
    source_digest: Digest,
    target: &Candidate,
    target_digest: Digest,
    receipt: &CandidateTransitionReceipt,
) -> Result<CandidateTransitionReference, TuneError> {
    let plan_digest = AttemptRole::TrainingChallenger { attempt_index }.plan_digest(
        stage,
        target_digest,
        session.fixed_seed,
    )?;
    let planning_context_digest =
        crate::adapter::planning_context_digest(session.stage_digest, plan_digest)?;
    let request = crate::CandidateTransitionRequest::new(
        super::storage::document_digest("session identity", session)?,
        source,
        source_digest,
        target,
        target_digest,
        session.runtimes.transition_validator.clone(),
        session.runtimes.adjacency_policy_digest,
        planning_context_digest,
    )?;
    receipt.validate_for(&request)?;
    Ok(receipt.reference())
}
