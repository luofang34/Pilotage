use flight_tune::{
    AttemptRole, AuthenticatedJournalRecord, CandidateEvaluation, CandidateTransitionReference,
    Digest, JournalEvent, OperationStatus, PromotionClosure, PromotionDecision, SearchStage,
    SessionIdentity,
};

use crate::{FeedbackError, error::invalid};

use super::super::plan;

mod retry;
mod score;

use score::derived_mean_loss;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Searching,
    Frozen,
    PromotionClosed,
    Sealed,
}

struct PendingAttempt {
    trial_id: u64,
    role: AttemptRole,
    candidate: Digest,
    plan_digest: Digest,
    transition: Option<CandidateTransitionReference>,
    retry_index: u32,
    outcome: Option<PendingOutcome>,
    quarantined: bool,
    decided: bool,
    replacement: Option<AuthorizedRetry>,
}

/// The replacement one quarantined attempt is owed.
#[derive(Clone, Copy)]
struct AuthorizedRetry {
    replacement_trial_id: u64,
    retry_index: u32,
    role: AttemptRole,
    candidate: Digest,
    plan_digest: Digest,
    transition: Option<CandidateTransitionReference>,
}

#[derive(Clone, Copy)]
struct PendingOutcome {
    passed: bool,
    selected: bool,
    mean_loss: Option<f64>,
}

struct ReplayState<'a> {
    stage: &'a SearchStage,
    session: &'a SessionIdentity,
    phase: Phase,
    next_trial_id: u64,
    pending: Option<PendingAttempt>,
    training_baseline_done: bool,
    training_incumbent: Digest,
    training_incumbent_loss: Option<f64>,
    training_attempt_count: u64,
    authorized_retry: Option<AuthorizedRetry>,
    frozen_candidate: Option<Digest>,
    promotion_baseline_passed: Option<bool>,
    promotion_frozen_done: bool,
    final_done: bool,
    promotion_closure: Option<PromotionClosure>,
}

pub(super) struct ReplayedAuthority {
    pub(super) baseline_candidate: Digest,
    pub(super) frozen_candidate: Digest,
    pub(super) final_candidate: Option<Digest>,
}

pub(super) fn verify(
    chain: &[AuthenticatedJournalRecord],
    stage: &SearchStage,
    session: &SessionIdentity,
) -> Result<ReplayedAuthority, FeedbackError> {
    let mut state = ReplayState::new(stage, session);
    for record in chain.iter().skip(1) {
        state.apply(&record.entry.event)?;
    }
    if state.pending.is_some() || !matches!(state.phase, Phase::PromotionClosed | Phase::Sealed) {
        return Err(invalid("the campaign authority has no stable replay head"));
    }
    let frozen_candidate = state
        .frozen_candidate
        .ok_or_else(|| invalid("the campaign authority replay has no frozen candidate"))?;
    let final_candidate = state
        .promotion_closure
        .as_ref()
        .and_then(authorized_final_candidate);
    Ok(ReplayedAuthority {
        baseline_candidate: session.initial_candidate_digest,
        frozen_candidate,
        final_candidate,
    })
}

impl<'a> ReplayState<'a> {
    fn new(stage: &'a SearchStage, session: &'a SessionIdentity) -> Self {
        Self {
            stage,
            session,
            phase: Phase::Searching,
            next_trial_id: 0,
            pending: None,
            training_baseline_done: false,
            training_incumbent: session.initial_candidate_digest,
            training_incumbent_loss: None,
            training_attempt_count: 0,
            authorized_retry: None,
            frozen_candidate: None,
            promotion_baseline_passed: None,
            promotion_frozen_done: false,
            final_done: false,
            promotion_closure: None,
        }
    }

    fn apply(&mut self, event: &JournalEvent) -> Result<(), FeedbackError> {
        if self.authorized_retry.is_some() && !matches!(event, JournalEvent::AttemptPrepared { .. })
        {
            return Err(invalid(
                "an authorized retry must be followed by its replacement attempt",
            ));
        }
        match event {
            JournalEvent::Started { .. } => Err(invalid("the campaign authority has two starts")),
            JournalEvent::CandidateTransitionAuthorized { .. } => {
                self.require_idle(Phase::Searching)
            }
            JournalEvent::AttemptPrepared {
                trial_id,
                role,
                candidate,
                plan_digest,
                transition,
            } => self.prepare(
                *trial_id,
                *role,
                *candidate,
                *plan_digest,
                transition.is_some(),
            ),
            JournalEvent::RunPrepared { trial_id, .. }
            | JournalEvent::RunBound { trial_id, .. }
            | JournalEvent::RunTerminalIntentPrepared { trial_id, .. }
            | JournalEvent::RunTerminalReportRecorded { trial_id, .. }
            | JournalEvent::RunTerminalEvidenceFailureRecorded { trial_id, .. }
            | JournalEvent::RunCommitted { trial_id, .. } => self.require_run(Some(*trial_id)),
            JournalEvent::AttemptCompleted {
                trial_id,
                evaluation,
                proof,
                selected_as_training_incumbent,
            } => self.complete(
                *trial_id,
                evaluation,
                proof.is_some(),
                *selected_as_training_incumbent,
            ),
            JournalEvent::AttemptQuarantined {
                trial_id, proof, ..
            } => self.quarantine(*trial_id, proof.is_some()),
            JournalEvent::RetryAuthorized {
                source_trial_id,
                replacement_trial_id,
                retry_index,
                ..
            } => self.retry_authorized(*source_trial_id, *replacement_trial_id, *retry_index),
            JournalEvent::RetryExhausted {
                source_trial_id,
                retry_index,
                ..
            } => self.retry_exhausted(*source_trial_id, *retry_index),
            JournalEvent::CleanupRecorded { trial_id, cleanup } => self.cleanup(*trial_id, cleanup),
            JournalEvent::Frozen {
                baseline,
                candidate,
            } => self.freeze(*baseline, *candidate),
            JournalEvent::PromotionClosed { closure } => self.close_promotion(closure),
            JournalEvent::Sealed { candidate, .. } => self.seal(*candidate),
        }
    }

    fn prepare(
        &mut self,
        trial_id: u64,
        role: AttemptRole,
        candidate: Digest,
        plan_digest: Digest,
        has_transition: bool,
    ) -> Result<(), FeedbackError> {
        let expected_plan = plan::digest_for(self.stage, role, candidate, self.session.fixed_seed)?;
        if self.pending.is_some()
            || trial_id != self.next_trial_id
            || plan_digest != expected_plan
            || !self.role_allowed(role, candidate, has_transition)
        {
            return Err(invalid("an authority attempt preparation changed"));
        }
        let retry_index = self.derived_retry_index(trial_id, role, candidate, plan_digest)?;
        self.pending = Some(PendingAttempt {
            trial_id,
            role,
            candidate,
            plan_digest,
            transition: None,
            retry_index,
            outcome: None,
            quarantined: false,
            decided: false,
            replacement: None,
        });
        self.authorized_retry = None;
        self.next_trial_id = self.next_trial_id.wrapping_add(1);
        Ok(())
    }

    fn role_allowed(&self, role: AttemptRole, candidate: Digest, transition: bool) -> bool {
        match role {
            AttemptRole::TrainingBaseline => {
                self.phase == Phase::Searching
                    && !self.training_baseline_done
                    && candidate == self.session.initial_candidate_digest
                    && !transition
            }
            AttemptRole::TrainingChallenger { attempt_index } => {
                self.phase == Phase::Searching
                    && self.training_incumbent_loss.is_some()
                    && attempt_index == self.training_attempt_count
                    && transition
            }
            AttemptRole::PromotionBaseline => {
                self.phase == Phase::Frozen
                    && self.promotion_baseline_passed.is_none()
                    && candidate == self.session.initial_candidate_digest
                    && !transition
            }
            AttemptRole::PromotionFrozen => {
                self.phase == Phase::Frozen
                    && self.promotion_baseline_passed == Some(true)
                    && !self.promotion_frozen_done
                    && self.frozen_candidate == Some(candidate)
                    && !transition
            }
            AttemptRole::FinalQualification => {
                self.phase == Phase::PromotionClosed
                    && !self.final_done
                    && self
                        .promotion_closure
                        .as_ref()
                        .and_then(authorized_final_candidate)
                        == Some(candidate)
                    && !transition
            }
        }
    }

    fn complete(
        &mut self,
        trial_id: u64,
        evaluation: &CandidateEvaluation,
        has_proof: bool,
        selected: Option<bool>,
    ) -> Result<(), FeedbackError> {
        let role = self
            .pending
            .as_ref()
            .filter(|pending| pending.trial_id == trial_id && pending.outcome.is_none())
            .map(|pending| pending.role)
            .ok_or_else(|| invalid("an authority event has the wrong active attempt"))?;
        let mean_loss = derived_mean_loss(evaluation)?;
        let expected_selected = match role {
            AttemptRole::TrainingBaseline => Some(mean_loss.is_some()),
            AttemptRole::TrainingChallenger { .. } => Some(mean_loss.is_some_and(|loss| {
                self.training_incumbent_loss
                    .is_none_or(|incumbent| loss < incumbent)
            })),
            AttemptRole::PromotionBaseline
            | AttemptRole::PromotionFrozen
            | AttemptRole::FinalQualification => None,
        };
        let hidden = !matches!(
            role,
            AttemptRole::TrainingBaseline | AttemptRole::TrainingChallenger { .. }
        );
        if selected != expected_selected || has_proof != hidden {
            return Err(invalid("an authority attempt outcome changed"));
        }
        let pending = self.active_pending_mut(trial_id)?;
        pending.outcome = Some(PendingOutcome {
            passed: mean_loss.is_some(),
            selected: selected.unwrap_or(false),
            mean_loss,
        });
        Ok(())
    }

    fn quarantine(&mut self, trial_id: u64, has_proof: bool) -> Result<(), FeedbackError> {
        let pending = self.active_pending_mut(trial_id)?;
        let hidden = !matches!(
            pending.role,
            AttemptRole::TrainingBaseline | AttemptRole::TrainingChallenger { .. }
        );
        if has_proof != hidden {
            return Err(invalid("an authority quarantine proof changed"));
        }
        pending.outcome = Some(PendingOutcome {
            passed: false,
            selected: false,
            mean_loss: None,
        });
        pending.quarantined = true;
        Ok(())
    }

    fn cleanup(&mut self, trial_id: u64, cleanup: &OperationStatus) -> Result<(), FeedbackError> {
        if self
            .pending
            .as_ref()
            .is_none_or(|pending| pending.trial_id != trial_id || pending.outcome.is_none())
        {
            return Err(invalid("an authority cleanup changed its attempt"));
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.quarantined && !pending.decided)
        {
            return Err(invalid("a quarantined attempt has no retry decision"));
        }
        if matches!(cleanup, OperationStatus::Succeeded) {
            let pending = self
                .pending
                .take()
                .ok_or_else(|| invalid("an authority cleanup lost its attempt"))?;
            self.finalize(pending)?;
        }
        Ok(())
    }

    fn finalize(&mut self, pending: PendingAttempt) -> Result<(), FeedbackError> {
        // A replaced execution states no result. Recording one would let a
        // quarantine both close its partition slot and receive a replacement,
        // which is two outcomes for one experimental condition.
        if let Some(retry) = pending.replacement {
            self.authorized_retry = Some(retry);
            return Ok(());
        }
        let outcome = pending
            .outcome
            .ok_or_else(|| invalid("an authority cleanup lost its outcome"))?;
        match pending.role {
            AttemptRole::TrainingBaseline => {
                self.training_baseline_done = true;
                self.update_incumbent(pending.candidate, outcome);
            }
            AttemptRole::TrainingChallenger { .. } => {
                self.training_attempt_count = self.training_attempt_count.wrapping_add(1);
                self.update_incumbent(pending.candidate, outcome);
            }
            AttemptRole::PromotionBaseline => {
                self.promotion_baseline_passed = Some(outcome.passed);
            }
            AttemptRole::PromotionFrozen => self.promotion_frozen_done = true,
            AttemptRole::FinalQualification => self.final_done = true,
        }
        Ok(())
    }

    fn update_incumbent(&mut self, candidate: Digest, outcome: PendingOutcome) {
        if outcome.selected {
            self.training_incumbent = candidate;
            self.training_incumbent_loss = outcome.mean_loss;
        }
    }

    fn freeze(&mut self, baseline: Digest, candidate: Digest) -> Result<(), FeedbackError> {
        self.require_idle(Phase::Searching)?;
        if !self.training_baseline_done
            || self.training_incumbent_loss.is_none()
            || baseline != self.session.initial_candidate_digest
            || candidate != self.training_incumbent
        {
            return Err(invalid(
                "the frozen candidate changed from training authority",
            ));
        }
        self.frozen_candidate = Some(candidate);
        self.phase = Phase::Frozen;
        Ok(())
    }

    fn close_promotion(&mut self, closure: &PromotionClosure) -> Result<(), FeedbackError> {
        self.require_idle(Phase::Frozen)?;
        let baseline_passed = self
            .promotion_baseline_passed
            .ok_or_else(|| invalid("promotion closed without its baseline authority"))?;
        let expected_candidate = match closure.decision {
            PromotionDecision::Promoted {} => self.frozen_candidate,
            PromotionDecision::RejectedNoImprovement {} => {
                Some(self.session.initial_candidate_digest)
            }
            PromotionDecision::RejectedHardGate { .. }
            | PromotionDecision::Indeterminate { .. } => None,
        };
        if baseline_passed != self.promotion_frozen_done
            || closure.selected_candidate != expected_candidate
        {
            return Err(invalid("promotion closure changed its candidate authority"));
        }
        self.promotion_closure = Some(closure.clone());
        self.phase = Phase::PromotionClosed;
        Ok(())
    }

    fn seal(&mut self, candidate: Digest) -> Result<(), FeedbackError> {
        self.require_idle(Phase::PromotionClosed)?;
        if !self.final_done
            || self
                .promotion_closure
                .as_ref()
                .and_then(authorized_final_candidate)
                != Some(candidate)
        {
            return Err(invalid(
                "the sealed candidate changed from promotion authority",
            ));
        }
        self.phase = Phase::Sealed;
        Ok(())
    }

    fn active_pending_mut(&mut self, trial_id: u64) -> Result<&mut PendingAttempt, FeedbackError> {
        self.pending
            .as_mut()
            .filter(|pending| pending.trial_id == trial_id && pending.outcome.is_none())
            .ok_or_else(|| invalid("an authority event has the wrong active attempt"))
    }

    fn require_run(&self, trial_id: Option<u64>) -> Result<(), FeedbackError> {
        if self
            .pending
            .as_ref()
            .is_none_or(|pending| Some(pending.trial_id) != trial_id || pending.outcome.is_some())
        {
            return Err(invalid("an authority run has the wrong active attempt"));
        }
        Ok(())
    }

    fn require_idle(&self, phase: Phase) -> Result<(), FeedbackError> {
        if self.pending.is_some() || self.phase != phase {
            return Err(invalid("the campaign authority phase changed"));
        }
        Ok(())
    }
}

fn authorized_final_candidate(closure: &PromotionClosure) -> Option<Digest> {
    match closure.decision {
        PromotionDecision::Promoted {} | PromotionDecision::RejectedNoImprovement {} => {
            closure.selected_candidate
        }
        PromotionDecision::RejectedHardGate { .. } | PromotionDecision::Indeterminate { .. } => {
            None
        }
    }
}
