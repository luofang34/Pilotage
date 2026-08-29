use crate::identity::digest_bytes;
use crate::{AttemptRole, CandidateTransitionReference, Digest, TuneError};

use super::{Journal, JournalEvent, invalid};

/// The domain that separates a quarantine reason from every other document.
const QUARANTINE_REASON_DOMAIN: &[u8] = b"pilotage.flight-tune.attempt-quarantine-reason.v1\0";

/// Returns the identity of one exact quarantine reason.
///
/// The identity covers the reason bytes themselves rather than any document
/// that carries them, so an independent verifier that holds the journal event
/// can recompute it without reading any other field.
#[must_use]
pub fn quarantine_reason_digest(reason: &str) -> Digest {
    let bytes = reason.as_bytes();
    let mut document =
        Vec::with_capacity(QUARANTINE_REASON_DOMAIN.len().saturating_add(bytes.len()));
    document.extend_from_slice(QUARANTINE_REASON_DOMAIN);
    document.extend_from_slice(bytes);
    digest_bytes(&document)
}

/// The replacement one quarantined attempt is owed.
///
/// The execution context travels with the authorization so that replay can
/// require the replacement to keep it. Nothing in the replacement event
/// itself decides the condition it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AuthorizedRetry {
    pub(crate) source_trial_id: u64,
    pub(crate) replacement_trial_id: u64,
    pub(crate) retry_index: u32,
    pub(crate) role: AttemptRole,
    pub(crate) candidate: Digest,
    pub(crate) plan_digest: Digest,
    pub(crate) transition: Option<CandidateTransitionReference>,
}

impl Journal {
    /// Saves the one authorized replacement for a quarantined attempt.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when no quarantined attempt awaits a decision or
    /// the declared limit does not permit a replacement.
    pub(crate) fn authorize_retry(&mut self, source_trial_id: u64) -> Result<(), TuneError> {
        let decision = self.pending_retry_decision(source_trial_id)?;
        if !decision.permits_replacement {
            return Err(invalid(
                "the execution retry limit does not permit a replacement",
            ));
        }
        self.append(JournalEvent::RetryAuthorized {
            source_trial_id,
            replacement_trial_id: decision.replacement_trial_id,
            retry_index: decision.replacement_retry_index,
            quarantine_reason_digest: decision.quarantine_reason_digest,
        })
    }

    /// Saves that a quarantined attempt reached its execution retry limit.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when no quarantined attempt awaits a decision or
    /// the declared limit still permits a replacement.
    pub(crate) fn exhaust_retry(&mut self, source_trial_id: u64) -> Result<(), TuneError> {
        let decision = self.pending_retry_decision(source_trial_id)?;
        if decision.permits_replacement {
            return Err(invalid(
                "the execution retry limit still permits a replacement",
            ));
        }
        self.append(JournalEvent::RetryExhausted {
            source_trial_id,
            retry_index: decision.source_retry_index,
            quarantine_reason_digest: decision.quarantine_reason_digest,
        })
    }

    /// Saves the decision the declared limit requires for a quarantined attempt.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when no quarantined attempt awaits a decision.
    pub(crate) fn record_retry_decision(&mut self, source_trial_id: u64) -> Result<(), TuneError> {
        if self
            .pending_retry_decision(source_trial_id)?
            .permits_replacement
        {
            self.authorize_retry(source_trial_id)
        } else {
            self.exhaust_retry(source_trial_id)
        }
    }

    /// Returns the replacement this journal still owes, if it owes one.
    pub(crate) const fn authorized_retry(&self) -> Option<AuthorizedRetry> {
        self.state.authorized_retry
    }

    /// Reports whether the pending attempt is quarantined without a decision.
    pub(crate) fn awaits_retry_decision(&self) -> Option<u64> {
        self.state
            .pending
            .as_ref()
            .filter(|pending| pending.awaits_retry_decision())
            .map(|pending| pending.trial_id)
    }

    /// Returns the derived retry index of one pending attempt.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the trial is not the pending attempt.
    pub(crate) fn pending_retry_index(&self, trial_id: u64) -> Result<u32, TuneError> {
        self.state
            .pending
            .as_ref()
            .filter(|pending| pending.trial_id == trial_id)
            .map(|pending| pending.retry_index)
            .ok_or_else(|| invalid("the trial is not the pending attempt"))
    }

    fn pending_retry_decision(&self, source_trial_id: u64) -> Result<RetryDecision, TuneError> {
        self.ensure_usable()?;
        let pending = self
            .state
            .pending
            .as_ref()
            .filter(|pending| {
                pending.trial_id == source_trial_id && pending.awaits_retry_decision()
            })
            .ok_or_else(|| invalid("no quarantined attempt awaits a retry decision"))?;
        let reason = pending
            .quarantine_reason()
            .ok_or_else(|| invalid("a quarantined attempt has no reason bytes"))?;
        Ok(RetryDecision {
            source_retry_index: pending.retry_index,
            replacement_retry_index: pending.retry_index.wrapping_add(1),
            replacement_trial_id: self.state.next_trial_id,
            permits_replacement: self
                .stage
                .execution_retry
                .permits_replacement(pending.retry_index),
            quarantine_reason_digest: quarantine_reason_digest(reason),
        })
    }
}

struct RetryDecision {
    source_retry_index: u32,
    replacement_retry_index: u32,
    replacement_trial_id: u64,
    permits_replacement: bool,
    quarantine_reason_digest: Digest,
}

#[cfg(test)]
mod tests;
