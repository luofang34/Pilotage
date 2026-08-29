use crate::{
    AttemptRole, AuthenticatedEvaluationProof, CandidateEvaluation, JournalEvent, OperationStatus,
    PromotionClosure, SearchStage, TuneError,
};

use super::{
    AuthenticatedJournalHead, AuthenticatedJournalRecord, CampaignEvidenceAuthority,
    JOURNAL_SCHEMA_VERSION, invalid, storage,
};

const MAX_AUTHORITY_CHAIN_ENTRIES: usize = 100_000;

pub(super) fn validate_chain(
    authority: &CampaignEvidenceAuthority,
    stage: &SearchStage,
    head: &AuthenticatedJournalHead,
    baseline: &AuthenticatedEvaluationProof,
    frozen: Option<&AuthenticatedEvaluationProof>,
    closure: &PromotionClosure,
    final_proof: Option<&AuthenticatedEvaluationProof>,
) -> Result<(), TuneError> {
    let chain = &authority.journal_chain;
    if chain.len() < 2 || chain.len() > MAX_AUTHORITY_CHAIN_ENTRIES {
        return Err(invalid("the campaign authority chain length is not valid"));
    }
    validate_chain_records(chain, head)?;
    validate_replay(
        chain,
        stage,
        authority,
        baseline,
        frozen,
        closure,
        final_proof,
    )?;
    validate_attempt_progression(chain)?;
    require_named_records(authority)?;
    require_promotion_closure(chain, closure)
}

#[allow(clippy::too_many_arguments)]
fn validate_replay(
    chain: &[AuthenticatedJournalRecord],
    stage: &SearchStage,
    authority: &CampaignEvidenceAuthority,
    baseline: &AuthenticatedEvaluationProof,
    frozen: Option<&AuthenticatedEvaluationProof>,
    closure: &PromotionClosure,
    final_proof: Option<&AuthenticatedEvaluationProof>,
) -> Result<(), TuneError> {
    let entries = chain
        .iter()
        .map(|record| record.entry.clone())
        .collect::<Vec<_>>();
    let digests = chain
        .iter()
        .map(|record| record.entry_digest)
        .collect::<Vec<_>>();
    let state = crate::journal::replay::replay(&entries, &digests, stage)?;
    if state.frozen_candidate != Some(authority.frozen_candidate)
        || state.promotion_baseline_proof.as_ref() != Some(baseline)
        || state.promotion_frozen_proof.as_ref() != frozen
        || state.promotion_closure.as_ref() != Some(closure)
        || state.final_proof.as_ref() != final_proof
    {
        return Err(invalid(
            "the campaign authority does not match independent journal replay",
        ));
    }
    Ok(())
}

fn validate_chain_records(
    chain: &[AuthenticatedJournalRecord],
    head: &AuthenticatedJournalHead,
) -> Result<(), TuneError> {
    let session = &head.entry.session;
    let mut previous = None;
    for (index, record) in chain.iter().enumerate() {
        let sequence = u64::try_from(index)
            .map_err(|_| invalid("the campaign authority sequence overflowed"))?;
        if record.entry.schema_version != JOURNAL_SCHEMA_VERSION
            || record.entry.sequence != sequence
            || record.entry.previous != previous
            || &record.entry.session != session
            || record.entry_digest.is_zero()
            || record.entry_digest != storage::document_digest("journal entry", &record.entry)?
        {
            return Err(invalid("the campaign authority chain changed"));
        }
        previous = Some(record.entry_digest);
    }
    let first = chain
        .first()
        .ok_or_else(|| invalid("the campaign authority chain is empty"))?;
    let last = chain
        .last()
        .ok_or_else(|| invalid("the campaign authority chain is empty"))?;
    if !matches!(
        &first.entry.event,
        JournalEvent::Started { candidate }
            if *candidate == session.initial_candidate_digest
    ) || last.entry != head.entry
        || last.entry_digest != head.entry_digest
    {
        return Err(invalid(
            "the campaign authority chain does not terminate at its head",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct PendingAttempt {
    trial_id: u64,
    outcome_saved: bool,
    quarantined: bool,
    decided: bool,
}

impl PendingAttempt {
    /// Reports whether this attempt still owes its one retry decision.
    const fn owes_decision(self) -> bool {
        self.quarantined && !self.decided
    }
}

fn validate_attempt_progression(chain: &[AuthenticatedJournalRecord]) -> Result<(), TuneError> {
    let mut next_trial_id = 0_u64;
    let mut pending: Option<PendingAttempt> = None;
    for record in chain {
        match &record.entry.event {
            JournalEvent::AttemptPrepared { trial_id, .. } => {
                if pending.is_some() || *trial_id != next_trial_id {
                    return Err(invalid("the campaign authority trial order changed"));
                }
                pending = Some(PendingAttempt {
                    trial_id: *trial_id,
                    outcome_saved: false,
                    quarantined: false,
                    decided: false,
                });
                next_trial_id = next_trial_id.wrapping_add(1);
            }
            event if run_trial(event).is_some() => {
                require_active_trial(pending, run_trial(event), false)?;
            }
            JournalEvent::AttemptCompleted { trial_id, .. } => {
                require_active_trial(pending, Some(*trial_id), false)?;
                pending = pending.map(|value| PendingAttempt {
                    outcome_saved: true,
                    ..value
                });
            }
            JournalEvent::AttemptQuarantined { trial_id, .. } => {
                require_active_trial(pending, Some(*trial_id), false)?;
                pending = pending.map(|value| PendingAttempt {
                    outcome_saved: true,
                    quarantined: true,
                    ..value
                });
            }
            JournalEvent::RetryAuthorized {
                source_trial_id, ..
            }
            | JournalEvent::RetryExhausted {
                source_trial_id, ..
            } => {
                require_active_trial(pending, Some(*source_trial_id), true)?;
                if pending.is_none_or(|value| !value.owes_decision()) {
                    return Err(invalid(
                        "a campaign authority retry answers no open quarantine",
                    ));
                }
                pending = pending.map(|value| PendingAttempt {
                    decided: true,
                    ..value
                });
            }
            JournalEvent::CleanupRecorded { trial_id, cleanup } => {
                require_active_trial(pending, Some(*trial_id), true)?;
                if pending.is_some_and(PendingAttempt::owes_decision) {
                    return Err(invalid(
                        "the campaign authority cleans a quarantine with no retry decision",
                    ));
                }
                if matches!(cleanup, OperationStatus::Succeeded) {
                    pending = None;
                }
            }
            JournalEvent::Frozen { .. }
            | JournalEvent::PromotionClosed { .. }
            | JournalEvent::Sealed { .. }
                if pending.is_some() =>
            {
                return Err(invalid("the campaign authority closes a pending attempt"));
            }
            _ => {}
        }
    }
    if pending.is_some() {
        return Err(invalid("the campaign authority has a pending attempt"));
    }
    Ok(())
}

fn run_trial(event: &JournalEvent) -> Option<u64> {
    match event {
        JournalEvent::RunPrepared { trial_id, .. }
        | JournalEvent::RunBound { trial_id, .. }
        | JournalEvent::RunTerminalIntentPrepared { trial_id, .. }
        | JournalEvent::RunTerminalReportRecorded { trial_id, .. }
        | JournalEvent::RunTerminalEvidenceFailureRecorded { trial_id, .. }
        | JournalEvent::RunCommitted { trial_id, .. } => Some(*trial_id),
        _ => None,
    }
}

fn require_active_trial(
    pending: Option<PendingAttempt>,
    trial_id: Option<u64>,
    outcome_required: bool,
) -> Result<(), TuneError> {
    if pending.is_none_or(|value| {
        Some(value.trial_id) != trial_id || value.outcome_saved != outcome_required
    }) {
        return Err(invalid(
            "a campaign authority event has the wrong active trial",
        ));
    }
    Ok(())
}

fn require_named_records(authority: &CampaignEvidenceAuthority) -> Result<(), TuneError> {
    let chain = &authority.journal_chain;
    match unique_record(chain, |event| matches!(event, JournalEvent::Frozen { .. }))? {
        Some(record) if record == &authority.frozen => {}
        _ => {
            return Err(invalid(
                "a named authority record is not in the journal chain",
            ));
        }
    }
    if last_attempt_record(chain, AttemptRole::PromotionBaseline)
        != Some(&authority.promotion_baseline)
    {
        return Err(invalid(
            "a named authority record is not in the journal chain",
        ));
    }
    require_settling_attempt(
        chain,
        authority.promotion_frozen.as_ref(),
        AttemptRole::PromotionFrozen,
    )?;
    require_settling_attempt(
        chain,
        authority.final_qualification.as_ref(),
        AttemptRole::FinalQualification,
    )
}

/// Requires the named record to be the preparation that settled its role.
///
/// A replaced execution prepares the same role again, so the chain may carry
/// several preparations for one hidden role. Only the last one produced the
/// result the evidence rests on.
fn require_settling_attempt(
    chain: &[AuthenticatedJournalRecord],
    saved: Option<&AuthenticatedJournalRecord>,
    role: AttemptRole,
) -> Result<(), TuneError> {
    if last_attempt_record(chain, role) != saved {
        return Err(invalid(
            "an optional authority record changed in the journal chain",
        ));
    }
    Ok(())
}

fn last_attempt_record(
    chain: &[AuthenticatedJournalRecord],
    expected_role: AttemptRole,
) -> Option<&AuthenticatedJournalRecord> {
    chain
        .iter()
        .filter(|record| {
            matches!(
                &record.entry.event,
                JournalEvent::AttemptPrepared { role, .. } if *role == expected_role
            )
        })
        .next_back()
}

fn require_promotion_closure(
    chain: &[AuthenticatedJournalRecord],
    closure: &PromotionClosure,
) -> Result<(), TuneError> {
    let Some(record) = unique_record(chain, |event| {
        matches!(event, JournalEvent::PromotionClosed { .. })
    })?
    else {
        return Err(invalid("the campaign authority has no promotion closure"));
    };
    let JournalEvent::PromotionClosed { closure: saved } = &record.entry.event else {
        return Err(invalid("the campaign authority closure event changed"));
    };
    if saved != closure {
        return Err(invalid("the campaign authority closure changed"));
    }
    Ok(())
}

fn unique_record(
    chain: &[AuthenticatedJournalRecord],
    predicate: impl Fn(&JournalEvent) -> bool,
) -> Result<Option<&AuthenticatedJournalRecord>, TuneError> {
    let mut records = chain.iter().filter(|record| predicate(&record.entry.event));
    let record = records.next();
    if records.next().is_some() {
        return Err(invalid(
            "the campaign authority chain repeats a required event",
        ));
    }
    Ok(record)
}

pub(super) fn validate_optional_proof(
    chain: &[AuthenticatedJournalRecord],
    attempt: Option<&AuthenticatedJournalRecord>,
    proof: Option<&AuthenticatedEvaluationProof>,
) -> Result<(), TuneError> {
    match (attempt, proof) {
        (Some(attempt), Some(proof)) => validate_proof(chain, attempt, proof),
        (None, None) => Ok(()),
        _ => Err(invalid(
            "an attempt proof lost its authenticated journal event",
        )),
    }
}

pub(super) fn validate_proof(
    chain: &[AuthenticatedJournalRecord],
    attempt: &AuthenticatedJournalRecord,
    proof: &AuthenticatedEvaluationProof,
) -> Result<(), TuneError> {
    let attempt_index = chain
        .iter()
        .position(|record| record == attempt)
        .ok_or_else(|| invalid("an attempt proof has no journal preparation"))?;
    let outcome_index = proof_outcome_index(chain, attempt_index, proof)?;
    validate_committed_receipts(&chain[attempt_index + 1..outcome_index], proof)?;
    validate_successful_cleanup(&chain[outcome_index + 1..], proof.trial_id)
}

fn proof_outcome_index(
    chain: &[AuthenticatedJournalRecord],
    attempt_index: usize,
    proof: &AuthenticatedEvaluationProof,
) -> Result<usize, TuneError> {
    let mut matches = chain
        .iter()
        .enumerate()
        .skip(attempt_index + 1)
        .filter(|(_, record)| outcome_matches(&record.entry.event, proof.trial_id));
    let Some((index, record)) = matches.next() else {
        return Err(invalid("an attempt proof has no journal outcome"));
    };
    if matches.next().is_some() || !exact_outcome_proof(&record.entry.event, proof) {
        return Err(invalid("an attempt proof changed from its journal outcome"));
    }
    Ok(index)
}

fn outcome_matches(event: &JournalEvent, trial_id: u64) -> bool {
    matches!(
        event,
        JournalEvent::AttemptCompleted { trial_id: saved, .. }
            | JournalEvent::AttemptQuarantined { trial_id: saved, .. }
            if *saved == trial_id
    )
}

fn exact_outcome_proof(event: &JournalEvent, proof: &AuthenticatedEvaluationProof) -> bool {
    match event {
        JournalEvent::AttemptCompleted {
            evaluation,
            proof: Some(saved),
            ..
        } => evaluation == &proof.evaluation && saved.as_ref() == proof,
        JournalEvent::AttemptQuarantined {
            reason,
            proof: Some(saved),
            ..
        } => {
            matches!(
                &proof.evaluation,
                CandidateEvaluation::Quarantined { reason: saved_reason }
                    if saved_reason == reason
            ) && saved.as_ref() == proof
        }
        _ => false,
    }
}

fn validate_committed_receipts(
    events: &[AuthenticatedJournalRecord],
    proof: &AuthenticatedEvaluationProof,
) -> Result<(), TuneError> {
    let mut committed = events
        .iter()
        .filter_map(|record| match &record.entry.event {
            JournalEvent::RunCommitted {
                trial_id,
                run_index,
                receipt,
            } if *trial_id == proof.trial_id => Some((*run_index, receipt.as_ref())),
            _ => None,
        });
    for (index, expected) in proof.terminal_receipts.iter().enumerate() {
        let Some((run_index, receipt)) = committed.next() else {
            return Err(invalid("an attempt proof lost a committed journal receipt"));
        };
        if u64::try_from(index) != Ok(run_index) || receipt != expected {
            return Err(invalid(
                "an attempt proof changed a committed journal receipt",
            ));
        }
    }
    if committed.next().is_some() {
        return Err(invalid(
            "an attempt proof changed its committed receipt count",
        ));
    }
    Ok(())
}

fn validate_successful_cleanup(
    events: &[AuthenticatedJournalRecord],
    trial_id: u64,
) -> Result<(), TuneError> {
    let mut cleanup = events.iter().filter(|record| {
        matches!(
            &record.entry.event,
            JournalEvent::CleanupRecorded {
                trial_id: saved,
                cleanup: OperationStatus::Succeeded,
            } if *saved == trial_id
        )
    });
    if cleanup.next().is_none() || cleanup.next().is_some() {
        return Err(invalid("an attempt proof has no unique successful cleanup"));
    }
    Ok(())
}
