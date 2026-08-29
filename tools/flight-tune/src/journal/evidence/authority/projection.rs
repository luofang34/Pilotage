use serde::{Deserialize, Serialize};

use crate::{AttemptRole, CandidateTransitionReference, Digest, JournalEvent, TuneError};

use super::super::super::retry::quarantine_reason_digest;
use super::{AuthenticatedJournalRecord, invalid};

/// The supported attempt projection schema.
pub const ATTEMPT_PROJECTION_SCHEMA_VERSION: u16 = 1;

/// What one quarantined attempt received in answer.
///
/// Every quarantine has exactly one of these. A quarantine with neither is an
/// execution the campaign never accounted for; one with both is an execution
/// counted twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "retry", rename_all = "snake_case", deny_unknown_fields)]
pub enum AttemptRetryOutcome {
    /// One replacement execution answered this quarantine.
    Authorized {
        /// The trial identity the replacement took.
        replacement_trial_id: u64,
        /// The retry index the replacement carried.
        replacement_retry_index: u32,
    },
    /// The declared limit permitted no replacement.
    Exhausted {
        /// The retry index the quarantined attempt carried.
        retry_index: u32,
    },
}

/// How one attempt ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum AttemptProjectionOutcome {
    /// The attempt produced a score or a hard gate result.
    Completed {
        /// The training incumbent decision, when this is a training role.
        selected_as_training_incumbent: Option<bool>,
    },
    /// The attempt was quarantined and received one retry decision.
    Quarantined {
        /// The identity of the exact journal reason bytes.
        reason_digest: Digest,
        /// The one runner receipt that quarantined this attempt.
        runner_quarantine_receipt_digest: Digest,
        /// The one decision the declared limit produced.
        retry: AttemptRetryOutcome,
    },
}

/// One journal-derived attempt identity and its exact ordered receipts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptProjectionRecord {
    /// The monotonic trial identity.
    pub trial_id: u64,
    /// The evaluation role, which carries a challenger's attempt index.
    pub role: AttemptRole,
    /// The evaluated candidate identity.
    pub candidate_digest: Digest,
    /// The complete ordered run-plan identity.
    pub plan_digest: Digest,
    /// The exact training transition authorization, when this is a challenger.
    pub transition_authorization: Option<CandidateTransitionReference>,
    /// How many replacements separate this attempt from its first execution.
    pub retry_index: u32,
    /// The ordered run intent identities this attempt made durable.
    pub run_intent_digests: Vec<Digest>,
    /// The ordered terminal receipt identities this attempt committed.
    pub terminal_receipt_digests: Vec<Digest>,
    /// How the attempt ended.
    pub outcome: AttemptProjectionOutcome,
}

/// The complete ordered attempt and retry relation one journal chain states.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptProjection {
    /// The projection schema.
    pub schema_version: u16,
    /// The limit the stage declared for this campaign.
    pub execution_retry_limit: u32,
    /// Every attempt the chain prepared, in chain order.
    pub attempts: Vec<AttemptProjectionRecord>,
}

impl AttemptProjection {
    /// Derives the complete relation from one authenticated journal chain.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the chain does not state one complete
    /// relation.
    pub fn from_journal_chain(
        chain: &[AuthenticatedJournalRecord],
        execution_retry_limit: u32,
    ) -> Result<Self, TuneError> {
        from_chain(chain, execution_retry_limit)
    }

    /// Requires that this projection is exactly what the chain states.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the projection omits, invents, or changes
    /// any attempt, quarantine, or retry decision the chain carries.
    pub(super) fn validate(
        &self,
        chain: &[AuthenticatedJournalRecord],
        execution_retry_limit: u32,
    ) -> Result<(), TuneError> {
        if self.schema_version != ATTEMPT_PROJECTION_SCHEMA_VERSION {
            return Err(invalid("the attempt projection schema changed"));
        }
        let recomputed = from_chain(chain, execution_retry_limit)?;
        if *self != recomputed {
            return Err(invalid(
                "the attempt projection does not match its journal chain",
            ));
        }
        require_committed_runs(&self.attempts)?;
        require_retry_bijection(&self.attempts, execution_retry_limit)
    }
}

/// Recomputes the whole attempt and retry relation from chain bytes alone.
///
/// # Errors
///
/// Returns [`TuneError`] when the chain does not state one complete relation.
pub(super) fn from_chain(
    chain: &[AuthenticatedJournalRecord],
    execution_retry_limit: u32,
) -> Result<AttemptProjection, TuneError> {
    let mut attempts: Vec<AttemptProjectionRecord> = Vec::new();
    let mut open: Option<usize> = None;
    let mut owed: Option<u64> = None;
    for record in chain {
        apply(&record.entry.event, &mut attempts, &mut open, &mut owed)?;
    }
    if open.is_some() || owed.is_some() {
        return Err(invalid("the journal chain has an unfinished attempt"));
    }
    Ok(AttemptProjection {
        schema_version: ATTEMPT_PROJECTION_SCHEMA_VERSION,
        execution_retry_limit,
        attempts,
    })
}

fn apply(
    event: &JournalEvent,
    attempts: &mut Vec<AttemptProjectionRecord>,
    open: &mut Option<usize>,
    owed: &mut Option<u64>,
) -> Result<(), TuneError> {
    match event {
        JournalEvent::AttemptPrepared {
            trial_id,
            role,
            candidate,
            plan_digest,
            transition,
        } => {
            if open.is_some() {
                return Err(invalid("a projection attempt opened inside another"));
            }
            let retry_index = replacement_retry_index(attempts, *trial_id, owed.take())?;
            *open = Some(attempts.len());
            attempts.push(AttemptProjectionRecord {
                trial_id: *trial_id,
                role: *role,
                candidate_digest: *candidate,
                plan_digest: *plan_digest,
                transition_authorization: *transition,
                retry_index,
                run_intent_digests: Vec::new(),
                terminal_receipt_digests: Vec::new(),
                outcome: AttemptProjectionOutcome::Completed {
                    selected_as_training_incumbent: None,
                },
            });
            Ok(())
        }
        JournalEvent::RunPrepared {
            context,
            run_intent_digest,
            ..
        } => {
            // The stated identity has to be the identity of the context that
            // states it, or a changed condition could travel under the
            // identity of the condition it replaced.
            if context.digest()? != *run_intent_digest {
                return Err(invalid(
                    "a prepared run identity does not cover its execution context",
                ));
            }
            open_record(attempts, *open)?
                .run_intent_digests
                .push(*run_intent_digest);
            Ok(())
        }
        JournalEvent::RunCommitted { receipt, .. } => {
            open_record(attempts, *open)?
                .terminal_receipt_digests
                .push(receipt.receipt_digest());
            Ok(())
        }
        JournalEvent::AttemptCompleted {
            selected_as_training_incumbent,
            ..
        } => {
            open_record(attempts, *open)?.outcome = AttemptProjectionOutcome::Completed {
                selected_as_training_incumbent: *selected_as_training_incumbent,
            };
            Ok(())
        }
        JournalEvent::AttemptQuarantined { reason, .. } => {
            let record = open_record(attempts, *open)?;
            let last = record
                .terminal_receipt_digests
                .last()
                .copied()
                .ok_or_else(|| invalid("a projection quarantine has no runner receipt"))?;
            record.outcome = AttemptProjectionOutcome::Quarantined {
                reason_digest: quarantine_reason_digest(reason),
                runner_quarantine_receipt_digest: last,
                // The decision arrives in the next event and replaces this.
                retry: AttemptRetryOutcome::Exhausted { retry_index: 0 },
            };
            Ok(())
        }
        JournalEvent::RetryAuthorized {
            source_trial_id,
            replacement_trial_id,
            retry_index,
            quarantine_reason_digest: reason_digest,
        } => {
            set_retry(
                attempts,
                *open,
                *source_trial_id,
                *reason_digest,
                AttemptRetryOutcome::Authorized {
                    replacement_trial_id: *replacement_trial_id,
                    replacement_retry_index: *retry_index,
                },
            )?;
            *owed = Some(*replacement_trial_id);
            Ok(())
        }
        JournalEvent::RetryExhausted {
            source_trial_id,
            retry_index,
            quarantine_reason_digest: reason_digest,
        } => set_retry(
            attempts,
            *open,
            *source_trial_id,
            *reason_digest,
            AttemptRetryOutcome::Exhausted {
                retry_index: *retry_index,
            },
        ),
        JournalEvent::CleanupRecorded { cleanup, .. } => {
            if cleanup.succeeded() {
                *open = None;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Derives the retry index one preparation must carry.
///
/// A first execution derives zero. A replacement derives one more than the
/// quarantined source that authorized it, read out of the projection rather
/// than out of the event being projected.
fn replacement_retry_index(
    attempts: &[AttemptProjectionRecord],
    trial_id: u64,
    owed: Option<u64>,
) -> Result<u32, TuneError> {
    let Some(expected) = owed else {
        return Ok(0);
    };
    if expected != trial_id {
        return Err(invalid(
            "a projection replacement changed its trial identity",
        ));
    }
    let source = attempts
        .iter()
        .rev()
        .find(|record| {
            matches!(
                record.outcome,
                AttemptProjectionOutcome::Quarantined {
                    retry: AttemptRetryOutcome::Authorized {
                        replacement_trial_id,
                        ..
                    },
                    ..
                } if replacement_trial_id == trial_id
            )
        })
        .ok_or_else(|| invalid("a projection replacement has no authorizing source"))?;
    Ok(source.retry_index.wrapping_add(1))
}

fn set_retry(
    attempts: &mut [AttemptProjectionRecord],
    open: Option<usize>,
    source_trial_id: u64,
    reason_digest: Digest,
    retry: AttemptRetryOutcome,
) -> Result<(), TuneError> {
    let record = open_record(attempts, open)?;
    if record.trial_id != source_trial_id {
        return Err(invalid("a projection retry names another attempt"));
    }
    let AttemptProjectionOutcome::Quarantined {
        reason_digest: saved,
        runner_quarantine_receipt_digest,
        ..
    } = record.outcome
    else {
        return Err(invalid("a projection retry answers no quarantine"));
    };
    if saved != reason_digest {
        return Err(invalid("a projection retry changed its quarantine reason"));
    }
    record.outcome = AttemptProjectionOutcome::Quarantined {
        reason_digest: saved,
        runner_quarantine_receipt_digest,
        retry,
    };
    Ok(())
}

fn open_record(
    attempts: &mut [AttemptProjectionRecord],
    open: Option<usize>,
) -> Result<&mut AttemptProjectionRecord, TuneError> {
    let index = open.ok_or_else(|| invalid("a projection event has no open attempt"))?;
    attempts
        .get_mut(index)
        .ok_or_else(|| invalid("a projection lost its open attempt"))
}

/// Requires every prepared run to have committed exactly one receipt.
///
/// An attempt cannot close with a prepared run left uncommitted, so a
/// projection whose two counts differ has lost or gained a receipt.
fn require_committed_runs(attempts: &[AttemptProjectionRecord]) -> Result<(), TuneError> {
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
fn require_retry_bijection(
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
