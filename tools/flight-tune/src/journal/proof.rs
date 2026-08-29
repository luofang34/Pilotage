use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::identity::digest_bytes;
use crate::{
    AttemptRole, CandidateEvaluation, Digest, RunRecord, RunTerminalCompletion,
    RunTerminalDisposition, RunTerminalReceipt, RunTerminalSemanticOutcome, TuneError,
};

/// The supported authenticated evaluation proof schema.
pub const AUTHENTICATED_EVALUATION_PROOF_SCHEMA_VERSION: u16 = 2;

const EVALUATION_DOMAIN: &[u8] = b"pilotage.flight-tune.authenticated-evaluation.v1\0";
const PROOF_DOMAIN: &[u8] = b"pilotage.flight-tune.authenticated-evaluation-proof.v1\0";

/// One journal-bound evaluation and its exact terminal receipts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticatedEvaluationProof {
    /// The proof schema.
    pub schema_version: u16,
    /// The monotonic trial identity.
    pub trial_id: u64,
    /// The hidden evaluation role.
    pub role: AttemptRole,
    /// The evaluated candidate identity.
    pub candidate_digest: Digest,
    /// The complete ordered run-plan identity.
    pub plan_digest: Digest,
    /// How many replacements separate this attempt from its first execution.
    pub retry_index: u32,
    /// The semantic evaluation.
    pub evaluation: CandidateEvaluation,
    /// The exact ordered terminal receipts.
    pub terminal_receipts: Vec<RunTerminalReceipt>,
    /// The identity of the bound evaluation.
    pub evaluation_digest: Digest,
    /// The identity of the complete proof.
    pub proof_digest: Digest,
}

#[derive(Serialize)]
struct EvaluationDocument<'a> {
    schema_version: u16,
    trial_id: u64,
    role: AttemptRole,
    candidate_digest: Digest,
    plan_digest: Digest,
    retry_index: u32,
    evaluation: &'a CandidateEvaluation,
}

#[derive(Serialize)]
struct ProofDocument<'a> {
    schema_version: u16,
    evaluation_digest: Digest,
    receipt_digests: &'a [Digest],
}

impl AuthenticatedEvaluationProof {
    pub(crate) fn new(
        trial_id: u64,
        role: AttemptRole,
        candidate_digest: Digest,
        plan_digest: Digest,
        retry_index: u32,
        evaluation: CandidateEvaluation,
        terminal_receipts: Vec<RunTerminalReceipt>,
    ) -> Result<Self, TuneError> {
        let mut proof = Self {
            schema_version: AUTHENTICATED_EVALUATION_PROOF_SCHEMA_VERSION,
            trial_id,
            role,
            candidate_digest,
            plan_digest,
            retry_index,
            evaluation,
            terminal_receipts,
            evaluation_digest: Digest::from_bytes([0; 32]),
            proof_digest: Digest::from_bytes([0; 32]),
        };
        proof.evaluation_digest = proof.recompute_evaluation_digest()?;
        proof.proof_digest = proof.recompute_proof_digest()?;
        proof.validate()?;
        Ok(proof)
    }

    /// Validates the complete evaluation, receipt chain, and proof identity.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when an identity or semantic result differs.
    pub fn validate(&self) -> Result<(), TuneError> {
        if self.schema_version != AUTHENTICATED_EVALUATION_PROOF_SCHEMA_VERSION
            || !is_hidden_role(self.role)
            || self.candidate_digest.is_zero()
            || self.plan_digest.is_zero()
            || self.terminal_receipts.is_empty()
        {
            return Err(invalid(
                "an authenticated evaluation proof header is not valid",
            ));
        }
        self.evaluation.validate(self.role.scenario_set())?;
        self.validate_receipts()?;
        validate_evaluation_receipts(&self.evaluation, &self.terminal_receipts)?;
        if self.evaluation_digest.is_zero()
            || self.evaluation_digest != self.recompute_evaluation_digest()?
            || self.proof_digest.is_zero()
            || self.proof_digest != self.recompute_proof_digest()?
        {
            return Err(invalid("an authenticated evaluation proof digest changed"));
        }
        Ok(())
    }

    /// Recomputes the identity of the journal-bound evaluation.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when JSON encoding fails.
    pub fn recompute_evaluation_digest(&self) -> Result<Digest, TuneError> {
        domain_digest(
            EVALUATION_DOMAIN,
            &EvaluationDocument {
                schema_version: self.schema_version,
                trial_id: self.trial_id,
                role: self.role,
                candidate_digest: self.candidate_digest,
                plan_digest: self.plan_digest,
                retry_index: self.retry_index,
                evaluation: &self.evaluation,
            },
            "authenticated evaluation",
        )
    }

    /// Recomputes the identity of the complete proof.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when JSON encoding fails.
    pub fn recompute_proof_digest(&self) -> Result<Digest, TuneError> {
        let receipt_digests = self
            .terminal_receipts
            .iter()
            .map(RunTerminalReceipt::receipt_digest)
            .collect::<Vec<_>>();
        domain_digest(
            PROOF_DOMAIN,
            &ProofDocument {
                schema_version: self.schema_version,
                evaluation_digest: self.evaluation_digest,
                receipt_digests: &receipt_digests,
            },
            "authenticated evaluation proof",
        )
    }

    fn validate_receipts(&self) -> Result<(), TuneError> {
        let mut digests = HashSet::new();
        for receipt in &self.terminal_receipts {
            receipt.validate()?;
            let context = receipt.context();
            if context.trial_id() != self.trial_id
                || context.role() != self.role
                || context.candidate_digest() != self.candidate_digest
                || context.retry_index() != self.retry_index
                || !digests.insert(receipt.receipt_digest())
            {
                return Err(invalid(
                    "an authenticated evaluation receipt identity changed or repeated",
                ));
            }
        }
        Ok(())
    }
}

fn validate_evaluation_receipts(
    evaluation: &CandidateEvaluation,
    receipts: &[RunTerminalReceipt],
) -> Result<(), TuneError> {
    match evaluation {
        CandidateEvaluation::Passed { runs, .. } => validate_passed(runs, receipts),
        CandidateEvaluation::HardGateFailed {
            failure,
            completed_runs,
        } => validate_hard_gate(completed_runs, failure, receipts),
        CandidateEvaluation::Quarantined { reason } => validate_quarantine(reason, receipts),
    }
}

fn validate_passed(runs: &[RunRecord], receipts: &[RunTerminalReceipt]) -> Result<(), TuneError> {
    if runs.len() != receipts.len()
        || !runs
            .iter()
            .zip(receipts)
            .all(|(run, receipt)| completed_run(receipt).is_some_and(|saved| saved == run))
    {
        return Err(invalid(
            "a passing evaluation proof changed its run receipts",
        ));
    }
    Ok(())
}

fn validate_hard_gate(
    runs: &[RunRecord],
    failure: &crate::HardGateFailure,
    receipts: &[RunTerminalReceipt],
) -> Result<(), TuneError> {
    let Some((last, prefix)) = receipts.split_last() else {
        return Err(invalid("a hard-gate proof has no abort receipt"));
    };
    if runs.len() != prefix.len()
        || !runs
            .iter()
            .zip(prefix)
            .all(|(run, receipt)| completed_run(receipt).is_some_and(|saved| saved == run))
        || completed_hard_gate(last) != Some(failure)
    {
        return Err(invalid("a hard-gate evaluation proof changed its receipts"));
    }
    Ok(())
}

fn validate_quarantine(reason: &str, receipts: &[RunTerminalReceipt]) -> Result<(), TuneError> {
    let Some((last, prefix)) = receipts.split_last() else {
        return Err(invalid("a quarantine proof has no terminal receipt"));
    };
    if prefix
        .iter()
        .any(|receipt| completed_run(receipt).is_none())
        || !matches!(
            last.class().disposition(),
            RunTerminalDisposition::Quarantine { .. }
        )
        || reason != crate::journal::terminal_quarantine_reason(last)?
    {
        return Err(invalid(
            "a quarantine evaluation proof changed its receipts",
        ));
    }
    Ok(())
}

fn completed_run(receipt: &RunTerminalReceipt) -> Option<&RunRecord> {
    if !matches!(
        receipt.class().disposition(),
        RunTerminalDisposition::Completed {
            completion: RunTerminalCompletion::ScenarioComplete
        }
    ) {
        return None;
    }
    match receipt.intent().outcome() {
        RunTerminalSemanticOutcome::ScenarioComplete { run, .. } => Some(run),
        _ => None,
    }
}

fn completed_hard_gate(receipt: &RunTerminalReceipt) -> Option<&crate::HardGateFailure> {
    if !matches!(
        receipt.class().disposition(),
        RunTerminalDisposition::Completed {
            completion: RunTerminalCompletion::HardGateAbort
        }
    ) {
        return None;
    }
    match receipt.intent().outcome() {
        RunTerminalSemanticOutcome::HardGateAbort { failure, .. } => Some(failure),
        _ => None,
    }
}

fn is_hidden_role(role: AttemptRole) -> bool {
    matches!(
        role,
        AttemptRole::PromotionBaseline
            | AttemptRole::PromotionFrozen
            | AttemptRole::FinalQualification
    )
}

fn domain_digest<T: Serialize>(
    domain: &[u8],
    document: &T,
    name: &'static str,
) -> Result<Digest, TuneError> {
    let encoded = serde_json::to_vec(document).map_err(|source| TuneError::Encode {
        document: name,
        source,
    })?;
    let mut bytes = Vec::with_capacity(domain.len().saturating_add(encoded.len()));
    bytes.extend_from_slice(domain);
    bytes.extend_from_slice(&encoded);
    Ok(digest_bytes(&bytes))
}

fn invalid(detail: impl Into<String>) -> TuneError {
    TuneError::InvalidJournal {
        detail: detail.into(),
    }
}
