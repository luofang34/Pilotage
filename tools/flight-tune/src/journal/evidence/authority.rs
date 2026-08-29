use serde::{Deserialize, Serialize};

use crate::{
    AttemptRole, AuthenticatedEvaluationProof, Digest, JournalEntry, JournalEvent,
    PromotionClosure, PromotionDecision, SearchStage, TuneError,
};

use super::{AuthenticatedJournalHead, JOURNAL_SCHEMA_VERSION, Journal, invalid, storage};

mod ancestry;
mod projection;

pub use projection::{
    ATTEMPT_PROJECTION_SCHEMA_VERSION, AttemptProjection, AttemptProjectionOutcome,
    AttemptProjectionRecord, AttemptRetryOutcome,
};

/// The supported campaign evidence authority schema.
pub const CAMPAIGN_EVIDENCE_AUTHORITY_SCHEMA_VERSION: u16 = 3;

/// One historical journal record with its canonical identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticatedJournalRecord {
    /// The exact historical journal entry.
    pub entry: JournalEntry,
    /// The canonical historical entry identity.
    pub entry_digest: Digest,
}

impl AuthenticatedJournalRecord {
    fn validate(&self, session: &crate::SessionIdentity) -> Result<(), TuneError> {
        if self.entry.schema_version != JOURNAL_SCHEMA_VERSION
            || self.entry.sequence == 0
            || self.entry.previous.is_none_or(Digest::is_zero)
            || &self.entry.session != session
            || self.entry_digest.is_zero()
            || self.entry_digest != storage::document_digest("journal entry", &self.entry)?
        {
            return Err(invalid("an authenticated authority record changed"));
        }
        Ok(())
    }
}

/// Journal-derived authority for candidate and hidden attempt identities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignEvidenceAuthority {
    /// The authority schema.
    pub schema_version: u16,
    /// The complete authenticated journal chain from session start through the evidence head.
    pub journal_chain: Vec<AuthenticatedJournalRecord>,
    /// The complete ordered attempt, quarantine, and retry relation.
    pub attempts: AttemptProjection,
    /// The initial baseline candidate identity.
    pub baseline_candidate: Digest,
    /// The candidate fixed by the freeze event.
    pub frozen_candidate: Digest,
    /// The candidate authorized for final qualification, if one exists.
    pub final_candidate: Option<Digest>,
    /// The exact freeze record.
    pub frozen: AuthenticatedJournalRecord,
    /// The exact promotion baseline preparation.
    pub promotion_baseline: AuthenticatedJournalRecord,
    /// The exact frozen candidate preparation, if it exists.
    pub promotion_frozen: Option<AuthenticatedJournalRecord>,
    /// The exact final qualification preparation, if it exists.
    pub final_qualification: Option<AuthenticatedJournalRecord>,
}

impl CampaignEvidenceAuthority {
    pub(super) fn validate(
        &self,
        stage: &SearchStage,
        head: &AuthenticatedJournalHead,
        baseline: &AuthenticatedEvaluationProof,
        frozen: Option<&AuthenticatedEvaluationProof>,
        closure: &PromotionClosure,
        final_proof: Option<&AuthenticatedEvaluationProof>,
    ) -> Result<(), TuneError> {
        if self.schema_version != CAMPAIGN_EVIDENCE_AUTHORITY_SCHEMA_VERSION {
            return Err(invalid("the campaign evidence authority schema changed"));
        }
        ancestry::validate_chain(self, stage, head, baseline, frozen, closure, final_proof)?;
        self.attempts.validate(
            &self.journal_chain,
            stage.execution_retry.execution_retry_limit,
        )?;
        self.validate_records(head)?;
        self.validate_candidates(head, closure)?;
        self.validate_attempts(stage, head, baseline, frozen, final_proof)
    }

    fn validate_records(&self, head: &AuthenticatedJournalHead) -> Result<(), TuneError> {
        let session = &head.entry.session;
        self.frozen.validate(session)?;
        self.promotion_baseline.validate(session)?;
        for record in [
            self.promotion_frozen.as_ref(),
            self.final_qualification.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            record.validate(session)?;
        }
        if self.frozen.entry.sequence >= self.promotion_baseline.entry.sequence
            || self.promotion_baseline.entry.sequence >= head.entry.sequence
        {
            return Err(invalid("the campaign authority record order changed"));
        }
        validate_optional_order(
            &self.promotion_baseline,
            self.promotion_frozen.as_ref(),
            self.final_qualification.as_ref(),
            head.entry.sequence,
        )
    }

    fn validate_candidates(
        &self,
        head: &AuthenticatedJournalHead,
        closure: &PromotionClosure,
    ) -> Result<(), TuneError> {
        let JournalEvent::Frozen {
            baseline,
            candidate,
        } = self.frozen.entry.event
        else {
            return Err(invalid("the campaign authority has no freeze event"));
        };
        let expected_final = match closure.decision {
            PromotionDecision::Promoted { .. }
            | PromotionDecision::RejectedNoImprovement { .. } => closure.selected_candidate,
            PromotionDecision::RejectedHardGate { .. }
            | PromotionDecision::Indeterminate { .. } => None,
        };
        if self.baseline_candidate.is_zero()
            || self.frozen_candidate.is_zero()
            || self.baseline_candidate != head.entry.session.initial_candidate_digest
            || baseline != self.baseline_candidate
            || candidate != self.frozen_candidate
            || self.final_candidate != expected_final
        {
            return Err(invalid("the campaign candidate authority changed"));
        }
        Ok(())
    }

    fn validate_attempts(
        &self,
        stage: &SearchStage,
        head: &AuthenticatedJournalHead,
        baseline: &AuthenticatedEvaluationProof,
        frozen: Option<&AuthenticatedEvaluationProof>,
        final_proof: Option<&AuthenticatedEvaluationProof>,
    ) -> Result<(), TuneError> {
        validate_attempt(
            &self.promotion_baseline,
            baseline,
            AttemptRole::PromotionBaseline,
            self.baseline_candidate,
            stage,
            head.entry.session.fixed_seed,
        )?;
        ancestry::validate_proof(&self.journal_chain, &self.promotion_baseline, baseline)?;
        validate_optional_attempt(
            self.promotion_frozen.as_ref(),
            frozen,
            AttemptRole::PromotionFrozen,
            self.frozen_candidate,
            stage,
            head.entry.session.fixed_seed,
        )?;
        ancestry::validate_optional_proof(
            &self.journal_chain,
            self.promotion_frozen.as_ref(),
            frozen,
        )?;
        let candidate = self.final_candidate.unwrap_or(self.baseline_candidate);
        validate_optional_attempt(
            self.final_qualification.as_ref(),
            final_proof,
            AttemptRole::FinalQualification,
            candidate,
            stage,
            head.entry.session.fixed_seed,
        )?;
        ancestry::validate_optional_proof(
            &self.journal_chain,
            self.final_qualification.as_ref(),
            final_proof,
        )
    }
}

pub(super) fn from_journal(journal: &Journal) -> Result<CampaignEvidenceAuthority, TuneError> {
    let closure = journal
        .state
        .promotion_closure
        .as_ref()
        .ok_or_else(|| invalid("promotion has no authority closure"))?;
    let final_candidate =
        match closure.decision {
            PromotionDecision::Promoted { .. }
            | PromotionDecision::RejectedNoImprovement { .. } => closure.selected_candidate,
            PromotionDecision::RejectedHardGate { .. }
            | PromotionDecision::Indeterminate { .. } => None,
        };
    let journal_chain = journal
        .entries
        .iter()
        .zip(&journal.entry_digests)
        .map(|(entry, digest)| AuthenticatedJournalRecord {
            entry: entry.clone(),
            entry_digest: *digest,
        })
        .collect::<Vec<_>>();
    let attempts = projection::from_chain(
        &journal_chain,
        journal.stage.execution_retry.execution_retry_limit,
    )?;
    Ok(CampaignEvidenceAuthority {
        schema_version: CAMPAIGN_EVIDENCE_AUTHORITY_SCHEMA_VERSION,
        journal_chain,
        attempts,
        baseline_candidate: journal
            .entries
            .first()
            .ok_or_else(|| invalid("the journal has no session authority"))?
            .session
            .initial_candidate_digest,
        frozen_candidate: journal
            .state
            .frozen_candidate
            .ok_or_else(|| invalid("the journal has no frozen candidate authority"))?,
        final_candidate,
        frozen: required_record(journal, |event| {
            matches!(event, JournalEvent::Frozen { .. })
        })?,
        promotion_baseline: attempt_record(journal, AttemptRole::PromotionBaseline)
            .ok_or_else(|| invalid("promotion has no baseline attempt authority"))?,
        promotion_frozen: attempt_record(journal, AttemptRole::PromotionFrozen),
        final_qualification: attempt_record(journal, AttemptRole::FinalQualification),
    })
}

fn validate_attempt(
    record: &AuthenticatedJournalRecord,
    proof: &AuthenticatedEvaluationProof,
    expected_role: AttemptRole,
    expected_candidate: Digest,
    stage: &SearchStage,
    fixed_seed: u64,
) -> Result<(), TuneError> {
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
    let expected_plan = expected_role.plan_digest(stage, expected_candidate, fixed_seed)?;
    if *role != expected_role
        || *candidate != expected_candidate
        || transition.is_some()
        || *plan_digest != expected_plan
        || proof.trial_id != *trial_id
        || proof.role != *role
        || proof.candidate_digest != *candidate
        || proof.plan_digest != *plan_digest
    {
        return Err(invalid("an authenticated attempt authority changed"));
    }
    Ok(())
}

fn validate_optional_attempt(
    record: Option<&AuthenticatedJournalRecord>,
    proof: Option<&AuthenticatedEvaluationProof>,
    role: AttemptRole,
    candidate: Digest,
    stage: &SearchStage,
    fixed_seed: u64,
) -> Result<(), TuneError> {
    match (record, proof) {
        (Some(record), Some(proof)) => {
            validate_attempt(record, proof, role, candidate, stage, fixed_seed)
        }
        (None, None) => Ok(()),
        _ => Err(invalid("an attempt proof lost its journal authority")),
    }
}

fn validate_optional_order(
    baseline: &AuthenticatedJournalRecord,
    frozen: Option<&AuthenticatedJournalRecord>,
    final_record: Option<&AuthenticatedJournalRecord>,
    head_sequence: u64,
) -> Result<(), TuneError> {
    let promotion_end = frozen.unwrap_or(baseline).entry.sequence;
    if frozen.is_some_and(|record| record.entry.sequence <= baseline.entry.sequence)
        || final_record.is_some_and(|record| {
            record.entry.sequence <= promotion_end || record.entry.sequence >= head_sequence
        })
        || frozen.is_some_and(|record| record.entry.sequence >= head_sequence)
    {
        return Err(invalid("the hidden attempt authority order changed"));
    }
    Ok(())
}

/// Returns the preparation that settled one hidden role.
///
/// A replaced execution prepares the same role again, so the settling
/// preparation is the last one the chain carries rather than the only one.
fn attempt_record(
    journal: &Journal,
    expected_role: AttemptRole,
) -> Option<AuthenticatedJournalRecord> {
    journal
        .entries
        .iter()
        .zip(&journal.entry_digests)
        .rfind(|(entry, _)| {
            matches!(
                &entry.event,
                JournalEvent::AttemptPrepared { role, .. } if *role == expected_role
            )
        })
        .map(|(entry, digest)| AuthenticatedJournalRecord {
            entry: entry.clone(),
            entry_digest: *digest,
        })
}

fn required_record(
    journal: &Journal,
    predicate: impl Fn(&JournalEvent) -> bool,
) -> Result<AuthenticatedJournalRecord, TuneError> {
    optional_record(journal, predicate)?
        .ok_or_else(|| invalid("the journal has no required authority record"))
}

fn optional_record(
    journal: &Journal,
    predicate: impl Fn(&JournalEvent) -> bool,
) -> Result<Option<AuthenticatedJournalRecord>, TuneError> {
    let mut matches = journal
        .entries
        .iter()
        .zip(&journal.entry_digests)
        .filter(|(entry, _)| predicate(&entry.event));
    let record = matches
        .next()
        .map(|(entry, digest)| AuthenticatedJournalRecord {
            entry: entry.clone(),
            entry_digest: *digest,
        });
    if matches.next().is_some() {
        return Err(invalid("the journal has repeated authority records"));
    }
    Ok(record)
}
