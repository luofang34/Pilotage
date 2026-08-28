use serde::{Deserialize, Serialize};

use crate::{
    AttemptRole, AuthenticatedEvaluationProof, Digest, FinalQualificationOutcome, JournalEntry,
    JournalEvent, MissionReference, PromotionClosure, PromotionDecision, RunExecutionContext,
    ScenarioSet, SearchStage, TuneError,
};

use super::{JOURNAL_SCHEMA_VERSION, Journal, storage};

mod authority;

pub use authority::{
    AuthenticatedJournalRecord, CAMPAIGN_EVIDENCE_AUTHORITY_SCHEMA_VERSION,
    CampaignEvidenceAuthority,
};

/// The supported journal evidence snapshot schema.
pub const JOURNAL_EVIDENCE_SNAPSHOT_SCHEMA_VERSION: u16 = 3;

/// One authenticated current journal head.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticatedJournalHead {
    /// The exact current journal entry.
    pub entry: JournalEntry,
    /// The canonical current entry identity.
    pub entry_digest: Digest,
}

impl AuthenticatedJournalHead {
    /// Validates the current entry schema and canonical identity.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the entry or identity differs.
    pub fn validate(&self) -> Result<(), TuneError> {
        if self.entry.schema_version != JOURNAL_SCHEMA_VERSION
            || self.entry.sequence == 0
            || self.entry.previous.is_none_or(Digest::is_zero)
            || self.entry_digest.is_zero()
            || self.entry_digest != storage::document_digest("journal entry", &self.entry)?
        {
            return Err(invalid("the authenticated journal head changed"));
        }
        Ok(())
    }
}

/// A stable promotion or sealed journal head and its authenticated evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalEvidenceSnapshot {
    /// The evidence snapshot schema.
    pub schema_version: u16,
    /// The complete search stage used by replay.
    pub stage: SearchStage,
    /// The exact current journal head.
    pub head: AuthenticatedJournalHead,
    /// The journal-derived candidate and attempt authority.
    pub authority: CampaignEvidenceAuthority,
    /// The initial promotion proof.
    pub promotion_baseline: AuthenticatedEvaluationProof,
    /// The frozen promotion proof, when that attempt ran.
    pub promotion_frozen: Option<AuthenticatedEvaluationProof>,
    /// The replay-computed promotion closure.
    pub promotion_closure: PromotionClosure,
    /// The final qualification proof at a sealed head.
    pub final_proof: Option<AuthenticatedEvaluationProof>,
    /// The final qualification result at a sealed head.
    pub final_outcome: Option<FinalQualificationOutcome>,
}

impl JournalEvidenceSnapshot {
    /// Validates every embedded proof, closure anchor, and stable head field.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when evidence differs from the exact head.
    pub fn validate(&self) -> Result<(), TuneError> {
        if self.schema_version != JOURNAL_EVIDENCE_SNAPSHOT_SCHEMA_VERSION {
            return Err(invalid("the journal evidence snapshot schema changed"));
        }
        self.stage.validate()?;
        self.head.validate()?;
        self.authority.validate(
            &self.stage,
            &self.head,
            &self.promotion_baseline,
            self.promotion_frozen.as_ref(),
            &self.promotion_closure,
            self.final_proof.as_ref(),
        )?;
        self.promotion_baseline.validate()?;
        if let Some(proof) = &self.promotion_frozen {
            proof.validate()?;
        }
        if let Some(proof) = &self.final_proof {
            proof.validate()?;
        }
        self.promotion_closure.validate_for(&self.stage.promotion)?;
        self.validate_proof_plans()?;
        self.validate_session_bindings()?;
        self.validate_closure_anchors()?;
        self.validate_recomputed_closure()?;
        self.validate_head_event()
    }

    fn validate_recomputed_closure(&self) -> Result<(), TuneError> {
        let expected = super::replay::expected_promotion_closure_from_proofs(
            &self.stage,
            &self.head.entry.session,
            &self.promotion_baseline,
            self.promotion_frozen.as_ref(),
        )?;
        if self.promotion_closure != expected {
            return Err(invalid(
                "the promotion closure does not match its authenticated proofs",
            ));
        }
        Ok(())
    }

    fn validate_session_bindings(&self) -> Result<(), TuneError> {
        let session = &self.head.entry.session;
        if session.stage_digest != storage::document_digest("search stage", &self.stage)? {
            return Err(invalid("the evidence stage identity changed"));
        }
        let session_digest = session.digest()?;
        for proof in [
            Some(&self.promotion_baseline),
            self.promotion_frozen.as_ref(),
            self.final_proof.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            self.validate_receipt_plan(proof, session_digest)?;
        }
        Ok(())
    }

    fn validate_receipt_plan(
        &self,
        proof: &AuthenticatedEvaluationProof,
        session_digest: Digest,
    ) -> Result<(), TuneError> {
        let session = &self.head.entry.session;
        let set = proof.role.scenario_set();
        let scenarios = scenarios(&self.stage, set);
        let repetitions = self.stage.repetitions as usize;
        let expected_count = scenarios
            .len()
            .checked_mul(repetitions)
            .ok_or_else(|| invalid("an evidence run plan count overflowed"))?;
        if proof.terminal_receipts.len() > expected_count {
            return Err(invalid("an evidence proof exceeds its run plan"));
        }
        for (index, receipt) in proof.terminal_receipts.iter().enumerate() {
            let scenario = scenarios
                .get(index / repetitions)
                .ok_or_else(|| invalid("an evidence receipt exceeds its run plan"))?;
            let repetition = u32::try_from(index % repetitions)
                .map_err(|_| invalid("an evidence repetition exceeds u32"))?;
            let expected = RunExecutionContext::new(
                session_digest,
                proof.trial_id,
                proof.role,
                proof.candidate_digest,
                None,
                set,
                scenario,
                repetition,
                crate::model::derive_seed(session.fixed_seed, set, scenario, repetition),
            )?;
            if receipt.context() != &expected
                || receipt.intent().run_intent_digest() != expected.digest()?
                || receipt.binding().adapter() != &session.runtimes.vehicle
            {
                return Err(invalid(
                    "an evidence receipt changed its run plan or vehicle binding",
                ));
            }
        }
        Ok(())
    }

    fn validate_proof_plans(&self) -> Result<(), TuneError> {
        let session = &self.head.entry.session;
        self.validate_proof_plan(
            &self.promotion_baseline,
            AttemptRole::PromotionBaseline,
            self.authority.baseline_candidate,
        )?;
        if let Some(proof) = &self.promotion_frozen {
            self.validate_proof_plan(
                proof,
                AttemptRole::PromotionFrozen,
                self.authority.frozen_candidate,
            )?;
        }
        if let Some(proof) = &self.final_proof {
            let selected = self
                .authority
                .final_candidate
                .ok_or_else(|| invalid("a final proof has no authorized candidate"))?;
            self.validate_proof_plan(proof, AttemptRole::FinalQualification, selected)?;
        }
        match self.promotion_closure.decision {
            PromotionDecision::Promoted { .. } => {
                if self.promotion_closure.selected_candidate
                    != Some(self.authority.frozen_candidate)
                {
                    return Err(invalid(
                        "promotion did not select its frozen proof candidate",
                    ));
                }
            }
            PromotionDecision::RejectedNoImprovement { .. } => {
                if self.promotion_closure.selected_candidate
                    != Some(session.initial_candidate_digest)
                {
                    return Err(invalid("rejection did not select the initial candidate"));
                }
            }
            PromotionDecision::RejectedHardGate { .. }
            | PromotionDecision::Indeterminate { .. } => {}
        }
        Ok(())
    }

    fn validate_proof_plan(
        &self,
        proof: &AuthenticatedEvaluationProof,
        role: AttemptRole,
        candidate: Digest,
    ) -> Result<(), TuneError> {
        let session = &self.head.entry.session;
        let expected_plan = role.plan_digest(&self.stage, candidate, session.fixed_seed)?;
        super::replay::plan::validate_evaluation(
            &proof.evaluation,
            role,
            &self.stage,
            session.fixed_seed,
        )?;
        if proof.role != role
            || proof.candidate_digest != candidate
            || proof.plan_digest != expected_plan
        {
            return Err(invalid(
                "an evidence proof changed its role, candidate, or plan",
            ));
        }
        Ok(())
    }

    fn validate_closure_anchors(&self) -> Result<(), TuneError> {
        let frozen = self
            .promotion_frozen
            .as_ref()
            .map(|proof| (proof.evaluation_digest, proof.proof_digest));
        if self.promotion_closure.baseline_evaluation_digest
            != Some(self.promotion_baseline.evaluation_digest)
            || self.promotion_closure.baseline_proof_digest
                != Some(self.promotion_baseline.proof_digest)
            || self.promotion_closure.frozen_evaluation_digest != frozen.map(|anchor| anchor.0)
            || self.promotion_closure.frozen_proof_digest != frozen.map(|anchor| anchor.1)
        {
            return Err(invalid("the promotion closure proof anchors changed"));
        }
        Ok(())
    }

    fn validate_head_event(&self) -> Result<(), TuneError> {
        match &self.head.entry.event {
            JournalEvent::PromotionClosed { closure } => {
                if closure != &self.promotion_closure
                    || self.final_proof.is_some()
                    || self.final_outcome.is_some()
                {
                    return Err(invalid("an open final head has sealed evidence"));
                }
            }
            JournalEvent::Sealed {
                candidate,
                outcome,
                promotion_closure_digest,
                final_evaluation_digest,
                final_proof_digest,
            } => {
                let proof = self
                    .final_proof
                    .as_ref()
                    .ok_or_else(|| invalid("a sealed head has no final proof"))?;
                let expected_outcome =
                    crate::campaign::final_outcome(&self.stage, Some(&proof.evaluation));
                if *candidate != proof.candidate_digest
                    || *outcome != expected_outcome
                    || self.final_outcome.as_ref() != Some(&expected_outcome)
                    || *promotion_closure_digest != self.promotion_closure.closure_digest
                    || *final_evaluation_digest != proof.evaluation_digest
                    || *final_proof_digest != proof.proof_digest
                {
                    return Err(invalid("a sealed head changed its evidence anchors"));
                }
            }
            _ => return Err(invalid("the journal head is not a stable evidence head")),
        }
        Ok(())
    }
}

impl Journal {
    /// Returns authenticated evidence only at a promotion closure or sealed head.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the live journal or stable evidence is not valid.
    pub fn verified_evidence_snapshot(&self) -> Result<JournalEvidenceSnapshot, TuneError> {
        self.ensure_usable()?;
        let entry = self
            .entries
            .last()
            .cloned()
            .ok_or_else(|| invalid("the journal has no evidence head"))?;
        if !matches!(
            entry.event,
            JournalEvent::PromotionClosed { .. } | JournalEvent::Sealed { .. }
        ) {
            return Err(invalid("the current journal head is not stable evidence"));
        }
        let snapshot = JournalEvidenceSnapshot {
            schema_version: JOURNAL_EVIDENCE_SNAPSHOT_SCHEMA_VERSION,
            stage: self.stage.clone(),
            head: AuthenticatedJournalHead {
                entry,
                entry_digest: self
                    .entry_digests
                    .last()
                    .copied()
                    .ok_or_else(|| invalid("the journal has no evidence head identity"))?,
            },
            authority: authority::from_journal(self)?,
            promotion_baseline: self
                .state
                .promotion_baseline_proof
                .clone()
                .ok_or_else(|| invalid("promotion has no initial authenticated proof"))?,
            promotion_frozen: self.state.promotion_frozen_proof.clone(),
            promotion_closure: self
                .state
                .promotion_closure
                .clone()
                .ok_or_else(|| invalid("promotion has no authenticated closure"))?,
            final_proof: self.state.final_proof.clone(),
            final_outcome: self.state.final_outcome.clone(),
        };
        snapshot.validate()?;
        Ok(snapshot)
    }
}

fn invalid(detail: impl Into<String>) -> TuneError {
    TuneError::InvalidJournal {
        detail: detail.into(),
    }
}

fn scenarios(stage: &SearchStage, set: ScenarioSet) -> &[MissionReference] {
    match set {
        ScenarioSet::Training => &stage.training_scenarios,
        ScenarioSet::Promotion => &stage.promotion_scenarios,
        ScenarioSet::FinalQualification => &stage.final_qualification_scenarios,
    }
}
