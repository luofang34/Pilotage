use flight_tune::{
    AttemptRole, AuthenticatedEvaluationProof, AuthenticatedJournalRecord, CandidateEvaluation,
    JournalEvent, JournalEvidenceSnapshot, OperationStatus, PromotionClosure,
};

use crate::{FeedbackError, digest, error::invalid};

const JOURNAL_SCHEMA_VERSION: u32 = 6;
const MAX_AUTHORITY_CHAIN_ENTRIES: usize = 100_000;

pub(super) struct DerivedRecords<'a> {
    pub(super) frozen: &'a AuthenticatedJournalRecord,
    pub(super) promotion_baseline: &'a AuthenticatedJournalRecord,
    pub(super) promotion_frozen: Option<&'a AuthenticatedJournalRecord>,
    pub(super) final_qualification: Option<&'a AuthenticatedJournalRecord>,
}

pub(super) fn verify(
    snapshot: &JournalEvidenceSnapshot,
) -> Result<DerivedRecords<'_>, FeedbackError> {
    let chain = &snapshot.authority.journal_chain;
    if chain.len() < 2 || chain.len() > MAX_AUTHORITY_CHAIN_ENTRIES {
        return Err(invalid("the campaign authority chain length is not valid"));
    }
    verify_chain_records(chain, snapshot)?;
    let records = derive_records(chain)?;
    verify_named_records(snapshot, &records)?;
    verify_promotion_closure(chain, &snapshot.promotion_closure)?;
    verify_proof(
        chain,
        records.promotion_baseline,
        &snapshot.promotion_baseline,
    )?;
    verify_optional_proof(
        chain,
        records.promotion_frozen,
        snapshot.promotion_frozen.as_ref(),
    )?;
    verify_optional_proof(
        chain,
        records.final_qualification,
        snapshot.final_proof.as_ref(),
    )?;
    Ok(records)
}

fn verify_chain_records(
    chain: &[AuthenticatedJournalRecord],
    snapshot: &JournalEvidenceSnapshot,
) -> Result<(), FeedbackError> {
    let session = &snapshot.head.entry.session;
    let mut previous = None;
    for (index, record) in chain.iter().enumerate() {
        let sequence = u64::try_from(index)
            .map_err(|_| invalid("the campaign authority sequence overflowed"))?;
        if record.entry.schema_version != JOURNAL_SCHEMA_VERSION
            || record.entry.sequence != sequence
            || record.entry.previous != previous
            || &record.entry.session != session
            || record.entry_digest.is_zero()
            || record.entry_digest != digest::document("journal entry", &record.entry)?
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
    ) || last.entry != snapshot.head.entry
        || last.entry_digest != snapshot.head.entry_digest
    {
        return Err(invalid(
            "the campaign authority chain does not terminate at its head",
        ));
    }
    Ok(())
}

fn derive_records(
    chain: &[AuthenticatedJournalRecord],
) -> Result<DerivedRecords<'_>, FeedbackError> {
    let frozen = required_record(chain, |event| matches!(event, JournalEvent::Frozen { .. }))?;
    let promotion_baseline = last_attempt_record(chain, AttemptRole::PromotionBaseline)
        .ok_or_else(|| invalid("the campaign authority has no required journal event"))?;
    let promotion_frozen = last_attempt_record(chain, AttemptRole::PromotionFrozen);
    let final_qualification = last_attempt_record(chain, AttemptRole::FinalQualification);
    Ok(DerivedRecords {
        frozen,
        promotion_baseline,
        promotion_frozen,
        final_qualification,
    })
}

fn verify_named_records(
    snapshot: &JournalEvidenceSnapshot,
    records: &DerivedRecords<'_>,
) -> Result<(), FeedbackError> {
    let saved = &snapshot.authority;
    if &saved.frozen != records.frozen
        || &saved.promotion_baseline != records.promotion_baseline
        || saved.promotion_frozen.as_ref() != records.promotion_frozen
        || saved.final_qualification.as_ref() != records.final_qualification
    {
        return Err(invalid(
            "a named authority record changed from its journal chain",
        ));
    }
    Ok(())
}

fn verify_promotion_closure(
    chain: &[AuthenticatedJournalRecord],
    closure: &PromotionClosure,
) -> Result<(), FeedbackError> {
    let record = required_record(chain, |event| {
        matches!(event, JournalEvent::PromotionClosed { .. })
    })?;
    let JournalEvent::PromotionClosed { closure: saved } = &record.entry.event else {
        return Err(invalid("the campaign authority closure event changed"));
    };
    if saved != closure {
        return Err(invalid("the campaign authority closure changed"));
    }
    Ok(())
}

/// Returns the preparation that settled one hidden role.
///
/// A replaced execution prepares the same role again, so the chain may carry
/// several preparations for one role. Only the last one produced the result
/// the evidence rests on.
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

fn required_record(
    chain: &[AuthenticatedJournalRecord],
    predicate: impl Fn(&JournalEvent) -> bool,
) -> Result<&AuthenticatedJournalRecord, FeedbackError> {
    unique_record(chain, predicate)?
        .ok_or_else(|| invalid("the campaign authority has no required journal event"))
}

fn unique_record(
    chain: &[AuthenticatedJournalRecord],
    predicate: impl Fn(&JournalEvent) -> bool,
) -> Result<Option<&AuthenticatedJournalRecord>, FeedbackError> {
    let mut records = chain.iter().filter(|record| predicate(&record.entry.event));
    let record = records.next();
    if records.next().is_some() {
        return Err(invalid(
            "the campaign authority repeats a required journal event",
        ));
    }
    Ok(record)
}

fn verify_optional_proof(
    chain: &[AuthenticatedJournalRecord],
    attempt: Option<&AuthenticatedJournalRecord>,
    proof: Option<&AuthenticatedEvaluationProof>,
) -> Result<(), FeedbackError> {
    match (attempt, proof) {
        (Some(attempt), Some(proof)) => verify_proof(chain, attempt, proof),
        (None, None) => Ok(()),
        _ => Err(invalid(
            "an attempt proof lost its authenticated journal event",
        )),
    }
}

fn verify_proof(
    chain: &[AuthenticatedJournalRecord],
    attempt: &AuthenticatedJournalRecord,
    proof: &AuthenticatedEvaluationProof,
) -> Result<(), FeedbackError> {
    let attempt_index = chain
        .iter()
        .position(|record| record == attempt)
        .ok_or_else(|| invalid("an attempt proof has no journal preparation"))?;
    let outcome_index = proof_outcome_index(chain, attempt_index, proof)?;
    verify_committed_receipts(&chain[attempt_index + 1..outcome_index], proof)?;
    verify_successful_cleanup(&chain[outcome_index + 1..], proof.trial_id)
}

fn proof_outcome_index(
    chain: &[AuthenticatedJournalRecord],
    attempt_index: usize,
    proof: &AuthenticatedEvaluationProof,
) -> Result<usize, FeedbackError> {
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

fn verify_committed_receipts(
    events: &[AuthenticatedJournalRecord],
    proof: &AuthenticatedEvaluationProof,
) -> Result<(), FeedbackError> {
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

fn verify_successful_cleanup(
    events: &[AuthenticatedJournalRecord],
    trial_id: u64,
) -> Result<(), FeedbackError> {
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
