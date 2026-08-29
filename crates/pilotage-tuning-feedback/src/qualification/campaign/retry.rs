//! Independent recomputation of the attempt, quarantine, and retry relation.
//!
//! Nothing here reads a count, an index, or a decision out of the campaign
//! document. Every value is derived again from the authenticated chain bytes
//! and the stored projection is then required to be exactly that.

use flight_tune::{
    AttemptProjection, AttemptProjectionOutcome, AttemptProjectionRecord, AttemptRetryOutcome,
    AttemptRole, AuthenticatedJournalRecord, CandidateTransitionReference, Digest, JournalEvent,
    OperationStatus, RunExecutionContext,
};

use crate::{FeedbackError, digest, error::invalid};

/// The projection schema this verifier reproduces.
const PROJECTION_SCHEMA_VERSION: u16 = 1;

/// The domain the core binds one run execution context under.
const RUN_CONTEXT_DOMAIN: &[u8] = b"flight-tune:run-execution-context:v4\0";

/// The domain the core binds one quarantine reason under.
const QUARANTINE_REASON_DOMAIN: &[u8] = b"pilotage.flight-tune.attempt-quarantine-reason.v1\0";

/// The derived relation, ready for lookup by trial identity.
pub(in crate::qualification) struct VerifiedAttempts {
    attempts: Vec<AttemptProjectionRecord>,
}

impl VerifiedAttempts {
    /// Returns the derived retry index for one trial.
    pub(in crate::qualification) fn retry_index(
        &self,
        trial_id: u64,
    ) -> Result<u32, FeedbackError> {
        self.attempts
            .iter()
            .find(|record| record.trial_id == trial_id)
            .map(|record| record.retry_index)
            .ok_or_else(|| invalid("a trial has no derived attempt projection"))
    }
}

/// Recomputes the relation and requires the stored projection to equal it.
///
/// # Errors
///
/// Returns [`FeedbackError`] when the projection omits, invents, or changes
/// any attempt, quarantine, or retry decision the chain carries.
pub(in crate::qualification) fn verify(
    chain: &[AuthenticatedJournalRecord],
    stored: &AttemptProjection,
    execution_retry_limit: u32,
) -> Result<VerifiedAttempts, FeedbackError> {
    if stored.schema_version != PROJECTION_SCHEMA_VERSION {
        return Err(invalid("the attempt projection schema changed"));
    }
    let attempts = derive(chain)?;
    if stored.execution_retry_limit != execution_retry_limit || stored.attempts != attempts {
        return Err(invalid(
            "the attempt projection does not match its journal chain",
        ));
    }
    require_committed_runs(&attempts)?;
    require_bijection(&attempts, execution_retry_limit)?;
    Ok(VerifiedAttempts { attempts })
}

/// Walks the chain once and rebuilds every attempt record from its events.
fn derive(
    chain: &[AuthenticatedJournalRecord],
) -> Result<Vec<AttemptProjectionRecord>, FeedbackError> {
    let mut attempts: Vec<AttemptProjectionRecord> = Vec::new();
    let mut open: Option<usize> = None;
    let mut owed: Option<u64> = None;
    for record in chain {
        apply(&record.entry.event, &mut attempts, &mut open, &mut owed)?;
    }
    if open.is_some() || owed.is_some() {
        return Err(invalid("the journal chain has an unfinished attempt"));
    }
    Ok(attempts)
}

fn apply(
    event: &JournalEvent,
    attempts: &mut Vec<AttemptProjectionRecord>,
    open: &mut Option<usize>,
    owed: &mut Option<u64>,
) -> Result<(), FeedbackError> {
    match event {
        JournalEvent::AttemptPrepared {
            trial_id,
            role,
            candidate,
            plan_digest,
            transition,
        } => open_attempt(
            attempts,
            open,
            owed,
            *trial_id,
            *role,
            *candidate,
            *plan_digest,
            *transition,
        ),
        JournalEvent::RunPrepared {
            context,
            run_intent_digest,
            ..
        } => record_prepared_run(attempts, *open, context, *run_intent_digest),
        JournalEvent::RunCommitted { receipt, .. } => {
            record_committed_run(attempts, *open, receipt.receipt_digest())
        }
        JournalEvent::AttemptCompleted {
            selected_as_training_incumbent,
            ..
        } => record_completion(attempts, *open, *selected_as_training_incumbent),
        JournalEvent::AttemptQuarantined { reason, .. } => {
            record_quarantine(attempts, *open, reason)
        }
        JournalEvent::RetryAuthorized {
            source_trial_id,
            replacement_trial_id,
            retry_index,
            quarantine_reason_digest,
        } => record_authorized_retry(
            attempts,
            *open,
            owed,
            *source_trial_id,
            *replacement_trial_id,
            *retry_index,
            *quarantine_reason_digest,
        ),
        JournalEvent::RetryExhausted {
            source_trial_id,
            retry_index,
            quarantine_reason_digest,
        } => set_retry(
            attempts,
            *open,
            *source_trial_id,
            *quarantine_reason_digest,
            AttemptRetryOutcome::Exhausted {
                retry_index: *retry_index,
            },
        ),
        JournalEvent::CleanupRecorded { cleanup, .. } => {
            if matches!(cleanup, OperationStatus::Succeeded) {
                *open = None;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Records one committed terminal receipt against its open attempt.
fn record_committed_run(
    attempts: &mut [AttemptProjectionRecord],
    open: Option<usize>,
    receipt_digest: Digest,
) -> Result<(), FeedbackError> {
    open_record(attempts, open)?
        .terminal_receipt_digests
        .push(receipt_digest);
    Ok(())
}

/// Records one completed outcome and the training decision it carried.
fn record_completion(
    attempts: &mut [AttemptProjectionRecord],
    open: Option<usize>,
    selected_as_training_incumbent: Option<bool>,
) -> Result<(), FeedbackError> {
    open_record(attempts, open)?.outcome = AttemptProjectionOutcome::Completed {
        selected_as_training_incumbent,
    };
    Ok(())
}

/// Records the one replacement a quarantined attempt received.
#[allow(clippy::too_many_arguments)]
fn record_authorized_retry(
    attempts: &mut [AttemptProjectionRecord],
    open: Option<usize>,
    owed: &mut Option<u64>,
    source_trial_id: u64,
    replacement_trial_id: u64,
    replacement_retry_index: u32,
    reason: Digest,
) -> Result<(), FeedbackError> {
    set_retry(
        attempts,
        open,
        source_trial_id,
        reason,
        AttemptRetryOutcome::Authorized {
            replacement_trial_id,
            replacement_retry_index,
        },
    )?;
    *owed = Some(replacement_trial_id);
    Ok(())
}

/// Records one prepared run under the identity that covers its context.
///
/// A stated identity that does not cover the context stating it would let a
/// changed condition travel under the identity of the one it replaced.
fn record_prepared_run(
    attempts: &mut [AttemptProjectionRecord],
    open: Option<usize>,
    context: &RunExecutionContext,
    run_intent_digest: Digest,
) -> Result<(), FeedbackError> {
    if digest::domain("run execution context", RUN_CONTEXT_DOMAIN, context)? != run_intent_digest {
        return Err(invalid(
            "a prepared run identity does not cover its execution context",
        ));
    }
    open_record(attempts, open)?
        .run_intent_digests
        .push(run_intent_digest);
    Ok(())
}

/// Records one quarantine against the last receipt the runner committed.
fn record_quarantine(
    attempts: &mut [AttemptProjectionRecord],
    open: Option<usize>,
    reason: &str,
) -> Result<(), FeedbackError> {
    let record = open_record(attempts, open)?;
    let last = record
        .terminal_receipt_digests
        .last()
        .copied()
        .ok_or_else(|| invalid("a projection quarantine has no runner receipt"))?;
    record.outcome = AttemptProjectionOutcome::Quarantined {
        reason_digest: reason_digest(reason),
        runner_quarantine_receipt_digest: last,
        // The decision arrives in the next event and replaces this.
        retry: AttemptRetryOutcome::Exhausted { retry_index: 0 },
    };
    Ok(())
}

/// Opens one attempt record with the retry index its authorization derives.
#[allow(clippy::too_many_arguments)]
fn open_attempt(
    attempts: &mut Vec<AttemptProjectionRecord>,
    open: &mut Option<usize>,
    owed: &mut Option<u64>,
    trial_id: u64,
    role: AttemptRole,
    candidate_digest: Digest,
    plan_digest: Digest,
    transition_authorization: Option<CandidateTransitionReference>,
) -> Result<(), FeedbackError> {
    if open.is_some() {
        return Err(invalid("a projection attempt opened inside another"));
    }
    let retry_index = derived_retry_index(attempts, trial_id, owed.take())?;
    *open = Some(attempts.len());
    attempts.push(AttemptProjectionRecord {
        trial_id,
        role,
        candidate_digest,
        plan_digest,
        transition_authorization,
        retry_index,
        run_intent_digests: Vec::new(),
        terminal_receipt_digests: Vec::new(),
        outcome: AttemptProjectionOutcome::Completed {
            selected_as_training_incumbent: None,
        },
    });
    Ok(())
}

/// Derives the retry index one preparation must carry.
///
/// A first execution derives zero. A replacement derives one more than the
/// quarantined source that authorized it, read out of the derived relation
/// rather than out of the event being derived.
fn derived_retry_index(
    attempts: &[AttemptProjectionRecord],
    trial_id: u64,
    owed: Option<u64>,
) -> Result<u32, FeedbackError> {
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
        .find(|record| authorizes(record, trial_id))
        .ok_or_else(|| invalid("a projection replacement has no authorizing source"))?;
    Ok(source.retry_index.wrapping_add(1))
}

fn authorizes(record: &AttemptProjectionRecord, trial_id: u64) -> bool {
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
}

fn set_retry(
    attempts: &mut [AttemptProjectionRecord],
    open: Option<usize>,
    source_trial_id: u64,
    reason: Digest,
    retry: AttemptRetryOutcome,
) -> Result<(), FeedbackError> {
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
    if saved != reason {
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
) -> Result<&mut AttemptProjectionRecord, FeedbackError> {
    let index = open.ok_or_else(|| invalid("a projection event has no open attempt"))?;
    attempts
        .get_mut(index)
        .ok_or_else(|| invalid("a projection lost its open attempt"))
}

/// Requires every prepared run to have committed exactly one receipt.
///
/// An attempt cannot close with a prepared run left uncommitted, so a
/// projection whose two counts differ has lost or gained a receipt.
fn require_committed_runs(attempts: &[AttemptProjectionRecord]) -> Result<(), FeedbackError> {
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

/// Requires one exact answer for every quarantine, and none without one.
fn require_bijection(
    attempts: &[AttemptProjectionRecord],
    execution_retry_limit: u32,
) -> Result<(), FeedbackError> {
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
    let replacements = attempts
        .iter()
        .filter(|record| record.retry_index > 0)
        .count();
    if replacements != claimed.len() {
        return Err(invalid("a replacement attempt has no authorizing source"));
    }
    Ok(())
}

/// Requires a replacement to keep every field except the two it may change.
fn require_same_condition(
    attempts: &[AttemptProjectionRecord],
    source: &AttemptProjectionRecord,
    replacement_trial_id: u64,
) -> Result<(), FeedbackError> {
    let replacement = attempts
        .iter()
        .find(|record| record.trial_id == replacement_trial_id)
        .ok_or_else(|| invalid("an authorized replacement never ran"))?;
    if replacement.role != source.role
        || replacement.candidate_digest != source.candidate_digest
        || replacement.plan_digest != source.plan_digest
        || replacement.transition_authorization != source.transition_authorization
        || replacement.trial_id == source.trial_id
        || replacement.retry_index != source.retry_index.wrapping_add(1)
    {
        return Err(invalid("a replacement changed its experimental condition"));
    }
    Ok(())
}

/// Recomputes the identity of one exact quarantine reason from its bytes.
fn reason_digest(reason: &str) -> Digest {
    let bytes = reason.as_bytes();
    let mut document =
        Vec::with_capacity(QUARANTINE_REASON_DOMAIN.len().saturating_add(bytes.len()));
    document.extend_from_slice(QUARANTINE_REASON_DOMAIN);
    document.extend_from_slice(bytes);
    digest::hash(&document)
}
