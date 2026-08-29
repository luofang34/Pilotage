//! How the independent replay derives a replacement from its source.

use flight_tune::{AttemptRole, Digest};

use crate::{FeedbackError, error::invalid};

use super::{AuthorizedRetry, PendingAttempt, ReplayState};

impl ReplayState<'_> {
    /// Derives how many replacements separate this preparation from its first
    /// execution.
    ///
    /// A first execution derives zero. A replacement derives its index from
    /// the authorization the quarantined source produced, so no preparation
    /// can state its own place in a retry chain.
    pub(super) fn derived_retry_index(
        &self,
        trial_id: u64,
        role: AttemptRole,
        candidate: Digest,
        plan_digest: Digest,
    ) -> Result<u32, FeedbackError> {
        let Some(retry) = self.authorized_retry else {
            return Ok(0);
        };
        if retry.replacement_trial_id != trial_id
            || retry.role != role
            || retry.candidate != candidate
            || retry.plan_digest != plan_digest
            || retry.transition.is_some()
        {
            return Err(invalid(
                "a replacement attempt changed its authorized execution context",
            ));
        }
        Ok(retry.retry_index)
    }

    pub(super) fn retry_authorized(
        &mut self,
        source_trial_id: u64,
        replacement_trial_id: u64,
        retry_index: u32,
    ) -> Result<(), FeedbackError> {
        let expected_replacement = self.next_trial_id;
        let limit = self.stage.execution_retry.execution_retry_limit;
        let pending = self.quarantined_without_decision(source_trial_id)?;
        if pending.retry_index >= limit
            || replacement_trial_id != expected_replacement
            || retry_index != pending.retry_index.wrapping_add(1)
        {
            return Err(invalid(
                "an authorized retry does not match its quarantined attempt",
            ));
        }
        pending.replacement = Some(AuthorizedRetry {
            replacement_trial_id,
            retry_index,
            role: pending.role,
            candidate: pending.candidate,
            plan_digest: pending.plan_digest,
            transition: pending.transition,
        });
        pending.decided = true;
        Ok(())
    }

    pub(super) fn retry_exhausted(
        &mut self,
        source_trial_id: u64,
        retry_index: u32,
    ) -> Result<(), FeedbackError> {
        let limit = self.stage.execution_retry.execution_retry_limit;
        let pending = self.quarantined_without_decision(source_trial_id)?;
        if pending.retry_index < limit || retry_index != pending.retry_index {
            return Err(invalid(
                "an exhausted retry stopped before the declared limit",
            ));
        }
        pending.decided = true;
        Ok(())
    }

    fn quarantined_without_decision(
        &mut self,
        trial_id: u64,
    ) -> Result<&mut PendingAttempt, FeedbackError> {
        self.pending
            .as_mut()
            .filter(|pending| {
                pending.trial_id == trial_id && pending.quarantined && !pending.decided
            })
            .ok_or_else(|| invalid("no quarantined attempt awaits a retry decision"))
    }
}
