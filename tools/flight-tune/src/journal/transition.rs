use crate::journal::{AttemptRole, SessionIdentity};
use crate::{
    Candidate, CandidateTransitionReceipt, CandidateTransitionReference, Digest,
    RunExecutionContext, SearchGroupBinding, SearchStage, TuneError,
};

use super::{Journal, JournalEvent, storage};

#[derive(Debug, Clone)]
pub(crate) struct AuthorizedTrainingTransition {
    pub(crate) attempt_index: u64,
    pub(crate) candidate: Digest,
    pub(crate) group: SearchGroupBinding,
    pub(crate) reference: CandidateTransitionReference,
}

impl Journal {
    pub(crate) fn authorized_training_transition(&self) -> Option<&AuthorizedTrainingTransition> {
        self.state.authorized_transition.as_ref()
    }

    /// Authorizes one exact training transition and its derived suite.
    ///
    /// The journal derives the search group again from the stored incumbent
    /// and the proposed candidate, so a caller cannot state a suite that the
    /// parameter difference does not select.
    pub(crate) fn authorize_training_transition(
        &mut self,
        attempt_index: u64,
        reason: impl Into<String>,
        candidate: &Candidate,
        group: &SearchGroupBinding,
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
        if &self.stage.derive_search_group(&source_candidate, candidate)? != group {
            return Err(TuneError::InvalidCandidate {
                detail: "a transition states a group the candidate difference does not select"
                    .to_owned(),
            });
        }
        let reference = validate_authorization(
            self.session(),
            &self.stage,
            attempt_index,
            &source_candidate,
            self.state.training_incumbent,
            candidate,
            target,
            group,
            &receipt,
        )?;
        self.append(JournalEvent::CandidateTransitionAuthorized {
            attempt_index,
            reason: reason.into(),
            candidate: target,
            group: group.clone(),
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

/// Checks every recorded group binding against the stored candidates.
///
/// Journal replay reads digests, so it cannot compare two parameter maps. A
/// journal that opens again reads the exact candidates and derives the group
/// once more. A campaign that recorded a suite its own difference does not
/// select therefore cannot resume.
pub(super) fn audit_recorded_bindings(
    storage: &storage::JournalStorage,
    entries: &[super::JournalEntry],
    stage: &SearchStage,
) -> Result<(), TuneError> {
    for entry in entries {
        let JournalEvent::CandidateTransitionAuthorized {
            candidate,
            group,
            receipt,
            ..
        } = &entry.event
        else {
            continue;
        };
        let source = storage::read_candidate(storage, receipt.source_candidate_digest())?;
        let target = storage::read_candidate(storage, *candidate)?;
        if &stage.derive_search_group(&source, &target)? != group {
            return Err(TuneError::InvalidJournal {
                detail: "a recorded transition suite does not match its candidate difference"
                    .to_owned(),
            });
        }
    }
    Ok(())
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
    group: &SearchGroupBinding,
    receipt: &CandidateTransitionReceipt,
) -> Result<CandidateTransitionReference, TuneError> {
    let role = AttemptRole::TrainingChallenger {
        attempt_index,
        suite_index: group.suite_index,
    };
    let plan_digest = role.plan_digest(stage, target_digest, session.fixed_seed)?;
    let planning_context_digest =
        crate::adapter::planning_context_digest(session.stage_digest, plan_digest, group)?;
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
