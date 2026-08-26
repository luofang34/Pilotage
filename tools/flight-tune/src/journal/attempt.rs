use crate::{Candidate, CandidateEvaluation, CandidateTransitionReference, Digest, TuneError};

use super::{AttemptRole, Journal, JournalEvent, OperationStatus, storage};

impl Journal {
    pub(crate) fn prepare_attempt(
        &mut self,
        role: AttemptRole,
        candidate: &Candidate,
        plan_digest: Digest,
        transition: Option<CandidateTransitionReference>,
    ) -> Result<(u64, Digest), TuneError> {
        self.prepare_attempt_with_hook(role, candidate, plan_digest, transition, || {})
    }

    #[cfg(test)]
    pub(crate) fn prepare_attempt_with_before_authorization_for_test(
        &mut self,
        role: AttemptRole,
        candidate: &Candidate,
        plan_digest: Digest,
        transition: Option<CandidateTransitionReference>,
        before_authorization: impl FnOnce(),
    ) -> Result<(u64, Digest), TuneError> {
        self.prepare_attempt_with_hook(
            role,
            candidate,
            plan_digest,
            transition,
            before_authorization,
        )
    }

    fn prepare_attempt_with_hook(
        &mut self,
        role: AttemptRole,
        candidate: &Candidate,
        plan_digest: Digest,
        transition: Option<CandidateTransitionReference>,
        before_authorization: impl FnOnce(),
    ) -> Result<(u64, Digest), TuneError> {
        self.ensure_usable()?;
        candidate.validate()?;
        let candidate_digest = self.record_storage_result(storage::store_candidate(
            &self.storage,
            &self.writer,
            candidate,
        ))?;
        let trial_id = self.state.next_trial_id;
        self.append_with_hook(
            JournalEvent::AttemptPrepared {
                trial_id,
                role,
                candidate: candidate_digest,
                plan_digest,
                transition,
            },
            before_authorization,
        )?;
        Ok((trial_id, candidate_digest))
    }

    pub(crate) fn complete_attempt(
        &mut self,
        trial_id: u64,
        evaluation: CandidateEvaluation,
        selected: Option<bool>,
    ) -> Result<(), TuneError> {
        self.ensure_usable()?;
        let role = self.state.pending_role(trial_id)?;
        evaluation.validate(role.scenario_set())?;
        self.append(JournalEvent::AttemptCompleted {
            trial_id,
            evaluation,
            selected_as_training_incumbent: selected,
        })
    }

    pub(crate) fn quarantine_attempt(&mut self, trial_id: u64) -> Result<(), TuneError> {
        self.ensure_usable()?;
        let reason = self
            .state
            .pending
            .as_ref()
            .filter(|pending| pending.trial_id == trial_id && pending.outcome.is_none())
            .ok_or_else(|| super::invalid("the attempt is not pending or already has an outcome"))?
            .terminal_quarantine_reason()?;
        self.append(JournalEvent::AttemptQuarantined { trial_id, reason })
    }

    pub(crate) fn record_cleanup(
        &mut self,
        trial_id: u64,
        cleanup: OperationStatus,
    ) -> Result<(), TuneError> {
        self.ensure_usable()?;
        self.append(JournalEvent::CleanupRecorded { trial_id, cleanup })
    }
}
