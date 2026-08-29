//! The cardinality rules one attempt and retry relation has to satisfy.

use crate::TuneError;

use super::super::invalid;
use super::{AttemptProjectionOutcome, AttemptProjectionRecord, AttemptRetryOutcome};

/// Requires every prepared run to have committed exactly one receipt.
///
/// An attempt cannot close with a prepared run left uncommitted, so a
/// projection whose two counts differ has lost or gained a receipt.
pub(super) fn require_committed_runs(
    attempts: &[AttemptProjectionRecord],
) -> Result<(), TuneError> {
    if attempts
        .iter()
        .any(|record| record.run_intent_digests.len() != record.terminal_receipt_digests.len())
    {
        return Err(invalid(
            "an attempt committed a different receipt count than it prepared",
        ));
    }
    Ok(())
}

/// Requires one exact answer for every quarantine, and no answer without one.
pub(super) fn require_retry_bijection(
    attempts: &[AttemptProjectionRecord],
    execution_retry_limit: u32,
) -> Result<(), TuneError> {
    let mut claimed: Vec<u64> = Vec::new();
    for record in attempts {
        let AttemptProjectionOutcome::Quarantined { retry, .. } = record.outcome else {
            continue;
        };
        match retry {
            AttemptRetryOutcome::Authorized {
                replacement_trial_id,
                replacement_retry_index,
            } => {
                if record.retry_index >= execution_retry_limit
                    || replacement_retry_index != record.retry_index.wrapping_add(1)
                    || claimed.contains(&replacement_trial_id)
                {
                    return Err(invalid(
                        "an authorized replacement is over limit or claimed twice",
                    ));
                }
                claimed.push(replacement_trial_id);
                require_same_condition(attempts, record, replacement_trial_id)?;
            }
            AttemptRetryOutcome::Exhausted { retry_index } => {
                if retry_index != record.retry_index || record.retry_index < execution_retry_limit {
                    return Err(invalid(
                        "an exhausted retry stopped before the declared limit",
                    ));
                }
            }
        }
    }
    require_every_replacement_claimed(attempts, &claimed)
}

/// Requires a replacement to keep every field except the two it may change.
fn require_same_condition(
    attempts: &[AttemptProjectionRecord],
    source: &AttemptProjectionRecord,
    replacement_trial_id: u64,
) -> Result<(), TuneError> {
    let replacement = attempts
        .iter()
        .find(|record| record.trial_id == replacement_trial_id)
        .ok_or_else(|| invalid("an authorized replacement never ran"))?;
    if replacement.role != source.role
        || replacement.candidate_digest != source.candidate_digest
        || replacement.plan_digest != source.plan_digest
        || replacement.transition_authorization != source.transition_authorization
        || replacement.trial_id == source.trial_id
    {
        return Err(invalid("a replacement changed its experimental condition"));
    }
    Ok(())
}

/// Requires every attempt above retry index zero to be a claimed replacement.
fn require_every_replacement_claimed(
    attempts: &[AttemptProjectionRecord],
    claimed: &[u64],
) -> Result<(), TuneError> {
    let replacements = attempts
        .iter()
        .filter(|record| record.retry_index > 0)
        .count();
    if replacements != claimed.len() {
        return Err(invalid("a replacement attempt has no authorizing source"));
    }
    for trial_id in claimed {
        if !attempts
            .iter()
            .any(|record| record.trial_id == *trial_id && record.retry_index > 0)
        {
            return Err(invalid("an authorized replacement has no first execution"));
        }
    }
    Ok(())
}
