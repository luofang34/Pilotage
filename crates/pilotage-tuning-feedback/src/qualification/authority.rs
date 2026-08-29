use flight_tune::{
    AttemptRole, AuthenticatedJournalRecord, Digest, JournalEvent, JournalEvidenceSnapshot,
    PromotionDecision, SearchStage,
};

use crate::{FeedbackError, error::invalid};

use super::campaign::retry::{self, VerifiedAttempts};
use super::plan;

mod ancestry;
mod candidates;
mod journal_replay;

const AUTHORITY_SCHEMA_VERSION: u16 = 4;

#[derive(Clone, Copy)]
pub(super) struct AttemptAuthority {
    pub(super) trial_id: u64,
    pub(super) role: AttemptRole,
    pub(super) candidate: Digest,
    pub(super) plan_digest: Digest,
    pub(super) retry_index: u32,
}

pub(super) struct VerifiedAuthority {
    pub(super) baseline_candidate: Digest,
    pub(super) frozen_candidate: Digest,
    pub(super) final_candidate: Option<Digest>,
    pub(super) promotion_baseline: AttemptAuthority,
    pub(super) promotion_frozen: Option<AttemptAuthority>,
    pub(super) final_qualification: Option<AttemptAuthority>,
}

pub(super) fn verify(
    snapshot: &JournalEvidenceSnapshot,
) -> Result<VerifiedAuthority, FeedbackError> {
    let saved = &snapshot.authority;
    if saved.schema_version != AUTHORITY_SCHEMA_VERSION {
        return Err(invalid("the campaign evidence authority schema changed"));
    }
    let records = ancestry::verify(snapshot)?;
    // The relation is recomputed from chain bytes before any stored index is
    // read, so every retry index below is derived rather than trusted.
    let attempts = retry::verify(
        &snapshot.authority.journal_chain,
        &snapshot.authority.attempts,
        snapshot.stage.execution_retry.execution_retry_limit,
    )?;
    let verified_candidates = candidates::verify(
        &snapshot.authority.journal_chain,
        &snapshot.authority.candidates,
    )?;
    let replayed = journal_replay::verify(
        &snapshot.authority.journal_chain,
        &snapshot.stage,
        &snapshot.head.entry.session,
        &verified_candidates,
    )?;
    verify_order(
        records.frozen,
        records.promotion_baseline,
        records.promotion_frozen,
        records.final_qualification,
    )?;
    let frozen_candidate = verify_frozen_candidate(snapshot, records.frozen, &replayed)?;
    let baseline = attempt(
        records.promotion_baseline,
        AttemptRole::PromotionBaseline,
        saved.baseline_candidate,
        &snapshot.stage,
        &attempts,
    )?;
    let frozen = optional_attempt(
        records.promotion_frozen,
        AttemptRole::PromotionFrozen,
        frozen_candidate,
        &snapshot.stage,
        &attempts,
    )?;
    let final_attempt = optional_attempt(
        records.final_qualification,
        AttemptRole::FinalQualification,
        saved.final_candidate.unwrap_or(saved.baseline_candidate),
        &snapshot.stage,
        &attempts,
    )?;
    Ok(VerifiedAuthority {
        baseline_candidate: saved.baseline_candidate,
        frozen_candidate,
        final_candidate: saved.final_candidate,
        promotion_baseline: baseline,
        promotion_frozen: frozen,
        final_qualification: final_attempt,
    })
}

fn verify_order(
    frozen_record: &AuthenticatedJournalRecord,
    baseline_record: &AuthenticatedJournalRecord,
    frozen_attempt: Option<&AuthenticatedJournalRecord>,
    final_attempt: Option<&AuthenticatedJournalRecord>,
) -> Result<(), FeedbackError> {
    let freeze = frozen_record.entry.sequence;
    let baseline = baseline_record.entry.sequence;
    let promotion_end = frozen_attempt.map_or(baseline, |record| record.entry.sequence);
    if freeze >= baseline
        || frozen_attempt.is_some_and(|record| record.entry.sequence <= baseline)
        || final_attempt.is_some_and(|record| record.entry.sequence <= promotion_end)
    {
        return Err(invalid("the campaign authority record order changed"));
    }
    Ok(())
}

fn verify_frozen_candidate(
    snapshot: &JournalEvidenceSnapshot,
    frozen_record: &AuthenticatedJournalRecord,
    replayed: &journal_replay::ReplayedAuthority,
) -> Result<Digest, FeedbackError> {
    let saved = &snapshot.authority;
    let JournalEvent::Frozen {
        baseline,
        candidate,
    } = frozen_record.entry.event
    else {
        return Err(invalid("the campaign authority has no freeze event"));
    };
    let expected_final = match snapshot.promotion_closure.decision {
        PromotionDecision::Promoted {} | PromotionDecision::RejectedNoImprovement {} => {
            snapshot.promotion_closure.selected_candidate
        }
        PromotionDecision::RejectedHardGate { .. } | PromotionDecision::Indeterminate { .. } => {
            None
        }
    };
    if saved.baseline_candidate.is_zero()
        || saved.frozen_candidate.is_zero()
        || saved.baseline_candidate != snapshot.head.entry.session.initial_candidate_digest
        || baseline != saved.baseline_candidate
        || candidate != saved.frozen_candidate
        || replayed.baseline_candidate != saved.baseline_candidate
        || replayed.frozen_candidate != saved.frozen_candidate
        || replayed.final_candidate != saved.final_candidate
        || saved.final_candidate != expected_final
    {
        return Err(invalid("the campaign candidate authority changed"));
    }
    Ok(candidate)
}

fn attempt(
    record: &AuthenticatedJournalRecord,
    expected_role: AttemptRole,
    expected_candidate: Digest,
    stage: &SearchStage,
    attempts: &VerifiedAttempts,
) -> Result<AttemptAuthority, FeedbackError> {
    let JournalEvent::AttemptPrepared {
        trial_id,
        role,
        candidate,
        plan_digest,
        transition,
    } = &record.entry.event
    else {
        return Err(invalid("an attempt authority has the wrong event"));
    };
    let expected_plan = plan::digest_for(
        stage,
        expected_role,
        expected_candidate,
        record.entry.session.fixed_seed,
    )?;
    if *role != expected_role
        || *candidate != expected_candidate
        || transition.is_some()
        || *plan_digest != expected_plan
    {
        return Err(invalid("an authenticated attempt authority changed"));
    }
    Ok(AttemptAuthority {
        trial_id: *trial_id,
        role: *role,
        candidate: *candidate,
        plan_digest: *plan_digest,
        retry_index: attempts.retry_index(*trial_id)?,
    })
}

fn optional_attempt(
    record: Option<&AuthenticatedJournalRecord>,
    role: AttemptRole,
    candidate: Digest,
    stage: &SearchStage,
    attempts: &VerifiedAttempts,
) -> Result<Option<AttemptAuthority>, FeedbackError> {
    record
        .map(|record| attempt(record, role, candidate, stage, attempts))
        .transpose()
}
