use pilotage_trial::Digest;
use serde::{Deserialize, Serialize};

use crate::{ArtifactIdentity, Candidate, TuneError};

mod digest;

pub(crate) use digest::planning_context_digest;
use digest::{ReceiptDocument, receipt_digest, validate_candidate_digest};

/// The supported candidate-transition receipt schema.
pub const CANDIDATE_TRANSITION_RECEIPT_SCHEMA_VERSION: u16 = 1;

/// The exact input to one vehicle-specific candidate transition check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateTransitionRequest {
    schema_version: u16,
    session_digest: Digest,
    source: Candidate,
    source_candidate_digest: Digest,
    target: Candidate,
    target_candidate_digest: Digest,
    validator: ArtifactIdentity,
    adjacency_policy_digest: Digest,
    planning_context_digest: Digest,
}

/// An immutable vehicle-specific candidate transition authorization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateTransitionReceipt {
    schema_version: u16,
    session_digest: Digest,
    source_candidate_digest: Digest,
    target_candidate_digest: Digest,
    validator: ArtifactIdentity,
    adjacency_policy_digest: Digest,
    planning_context_digest: Digest,
    receipt_digest: Digest,
}

/// The exact receipt fields carried by a downstream execution identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateTransitionReference {
    schema_version: u16,
    session_digest: Digest,
    source_candidate_digest: Digest,
    target_candidate_digest: Digest,
    validator_digest: Digest,
    adjacency_policy_digest: Digest,
    planning_context_digest: Digest,
    receipt_digest: Digest,
}

impl CandidateTransitionRequest {
    /// Creates one complete transition check request.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when an identity is invalid, a candidate digest
    /// differs, or the candidates are not adjacent forms of one parameter set.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_digest: Digest,
        source: &Candidate,
        source_candidate_digest: Digest,
        target: &Candidate,
        target_candidate_digest: Digest,
        validator: ArtifactIdentity,
        adjacency_policy_digest: Digest,
        planning_context_digest: Digest,
    ) -> Result<Self, TuneError> {
        validate_request_identities(
            session_digest,
            source_candidate_digest,
            target_candidate_digest,
            &validator,
            adjacency_policy_digest,
            planning_context_digest,
        )?;
        source.validate()?;
        target.validate()?;
        validate_candidate_pair(source, target)?;
        validate_candidate_digest(source, source_candidate_digest, "source")?;
        validate_candidate_digest(target, target_candidate_digest, "target")?;
        Ok(Self {
            schema_version: CANDIDATE_TRANSITION_RECEIPT_SCHEMA_VERSION,
            session_digest,
            source: source.clone(),
            source_candidate_digest,
            target: target.clone(),
            target_candidate_digest,
            validator,
            adjacency_policy_digest,
            planning_context_digest,
        })
    }

    /// Returns the tuning session identity.
    #[must_use]
    pub const fn session_digest(&self) -> Digest {
        self.session_digest
    }

    /// Returns the exact source candidate.
    #[must_use]
    pub const fn source(&self) -> &Candidate {
        &self.source
    }

    /// Returns the source candidate identity.
    #[must_use]
    pub const fn source_candidate_digest(&self) -> Digest {
        self.source_candidate_digest
    }

    /// Returns the exact target candidate.
    #[must_use]
    pub const fn target(&self) -> &Candidate {
        &self.target
    }

    /// Returns the target candidate identity.
    #[must_use]
    pub const fn target_candidate_digest(&self) -> Digest {
        self.target_candidate_digest
    }

    /// Returns the transition validator identity.
    #[must_use]
    pub const fn validator(&self) -> &ArtifactIdentity {
        &self.validator
    }

    /// Returns the exact adjacency-policy identity.
    #[must_use]
    pub const fn adjacency_policy_digest(&self) -> Digest {
        self.adjacency_policy_digest
    }

    /// Returns the opaque planning-context identity.
    #[must_use]
    pub const fn planning_context_digest(&self) -> Digest {
        self.planning_context_digest
    }
}

impl CandidateTransitionReceipt {
    /// Creates a receipt after a vehicle adapter accepts the request.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when canonical receipt encoding fails.
    pub fn authorized(request: &CandidateTransitionRequest) -> Result<Self, TuneError> {
        validate_request(request)?;
        let mut receipt = Self {
            schema_version: request.schema_version,
            session_digest: request.session_digest,
            source_candidate_digest: request.source_candidate_digest,
            target_candidate_digest: request.target_candidate_digest,
            validator: request.validator.clone(),
            adjacency_policy_digest: request.adjacency_policy_digest,
            planning_context_digest: request.planning_context_digest,
            receipt_digest: Digest::from_bytes([0; 32]),
        };
        receipt.receipt_digest = receipt.recompute_digest()?;
        Ok(receipt)
    }

    /// Validates the receipt against the exact requested transition.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when a field or the canonical digest differs.
    pub fn validate_for(&self, request: &CandidateTransitionRequest) -> Result<(), TuneError> {
        let expected = Self::authorized(request)?;
        if self == &expected {
            Ok(())
        } else {
            Err(receipt_mismatch(
                "the transition receipt does not match the exact request",
            ))
        }
    }

    /// Recomputes the domain-separated canonical receipt identity.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when canonical receipt encoding fails.
    pub fn recompute_digest(&self) -> Result<Digest, TuneError> {
        receipt_digest(&ReceiptDocument {
            schema_version: self.schema_version,
            session_digest: self.session_digest,
            source_candidate_digest: self.source_candidate_digest,
            target_candidate_digest: self.target_candidate_digest,
            validator: &self.validator,
            adjacency_policy_digest: self.adjacency_policy_digest,
            planning_context_digest: self.planning_context_digest,
        })
    }

    /// Returns the receipt schema version.
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    /// Returns the tuning session identity.
    #[must_use]
    pub const fn session_digest(&self) -> Digest {
        self.session_digest
    }

    /// Returns the source candidate identity.
    #[must_use]
    pub const fn source_candidate_digest(&self) -> Digest {
        self.source_candidate_digest
    }

    /// Returns the target candidate identity.
    #[must_use]
    pub const fn target_candidate_digest(&self) -> Digest {
        self.target_candidate_digest
    }

    /// Returns the transition validator identity.
    #[must_use]
    pub const fn validator(&self) -> &ArtifactIdentity {
        &self.validator
    }

    /// Returns the adjacency-policy identity.
    #[must_use]
    pub const fn adjacency_policy_digest(&self) -> Digest {
        self.adjacency_policy_digest
    }

    /// Returns the opaque planning-context identity.
    #[must_use]
    pub const fn planning_context_digest(&self) -> Digest {
        self.planning_context_digest
    }

    /// Returns the complete receipt identity.
    #[must_use]
    pub const fn receipt_digest(&self) -> Digest {
        self.receipt_digest
    }

    /// Returns the downstream reference for this receipt.
    #[must_use]
    pub const fn reference(&self) -> CandidateTransitionReference {
        CandidateTransitionReference {
            schema_version: self.schema_version,
            session_digest: self.session_digest,
            source_candidate_digest: self.source_candidate_digest,
            target_candidate_digest: self.target_candidate_digest,
            validator_digest: self.validator.digest,
            adjacency_policy_digest: self.adjacency_policy_digest,
            planning_context_digest: self.planning_context_digest,
            receipt_digest: self.receipt_digest,
        }
    }
}

impl CandidateTransitionReference {
    /// Reports whether the reference has valid fields for one target.
    ///
    /// This check does not authorize the reference. Replay must also call
    /// [`Self::validate_for_runtime`] with the frozen runtime identities.
    #[must_use]
    pub fn is_valid_for_target(self, target_candidate_digest: Digest) -> bool {
        self.schema_version == CANDIDATE_TRANSITION_RECEIPT_SCHEMA_VERSION
            && !self.session_digest.is_zero()
            && !self.source_candidate_digest.is_zero()
            && self.target_candidate_digest == target_candidate_digest
            && !self.target_candidate_digest.is_zero()
            && self.source_candidate_digest != self.target_candidate_digest
            && !self.validator_digest.is_zero()
            && !self.adjacency_policy_digest.is_zero()
            && !self.planning_context_digest.is_zero()
            && !self.receipt_digest.is_zero()
    }

    /// Validates this reference against one runtime transition contract.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when a field or the canonical digest differs.
    pub fn validate_for_runtime(
        self,
        session_digest: Digest,
        source_candidate_digest: Digest,
        target_candidate_digest: Digest,
        validator: &ArtifactIdentity,
        adjacency_policy_digest: Digest,
        planning_context_digest: Digest,
    ) -> Result<(), TuneError> {
        validator.validate()?;
        let receipt = CandidateTransitionReceipt {
            schema_version: self.schema_version,
            session_digest,
            source_candidate_digest,
            target_candidate_digest,
            validator: validator.clone(),
            adjacency_policy_digest,
            planning_context_digest,
            receipt_digest: self.receipt_digest,
        };
        let expected = receipt.recompute_digest()?;
        if self.schema_version == CANDIDATE_TRANSITION_RECEIPT_SCHEMA_VERSION
            && self.session_digest == session_digest
            && !self.session_digest.is_zero()
            && self.source_candidate_digest == source_candidate_digest
            && !source_candidate_digest.is_zero()
            && self.target_candidate_digest == target_candidate_digest
            && !self.target_candidate_digest.is_zero()
            && self.validator_digest == validator.digest
            && self.adjacency_policy_digest == adjacency_policy_digest
            && !self.adjacency_policy_digest.is_zero()
            && self.planning_context_digest == planning_context_digest
            && !self.planning_context_digest.is_zero()
            && self.source_candidate_digest != self.target_candidate_digest
            && !self.receipt_digest.is_zero()
            && self.receipt_digest == expected
        {
            Ok(())
        } else {
            Err(receipt_mismatch(
                "the transition reference does not match the runtime contract",
            ))
        }
    }

    /// Returns the receipt schema version.
    #[must_use]
    pub const fn schema_version(self) -> u16 {
        self.schema_version
    }

    /// Returns the tuning session identity.
    #[must_use]
    pub const fn session_digest(self) -> Digest {
        self.session_digest
    }

    /// Returns the source candidate identity.
    #[must_use]
    pub const fn source_candidate_digest(self) -> Digest {
        self.source_candidate_digest
    }

    /// Returns the target candidate identity.
    #[must_use]
    pub const fn target_candidate_digest(self) -> Digest {
        self.target_candidate_digest
    }

    /// Returns the transition validator identity.
    #[must_use]
    pub const fn validator_digest(self) -> Digest {
        self.validator_digest
    }

    /// Returns the adjacency-policy identity.
    #[must_use]
    pub const fn adjacency_policy_digest(self) -> Digest {
        self.adjacency_policy_digest
    }

    /// Returns the opaque planning-context identity.
    #[must_use]
    pub const fn planning_context_digest(self) -> Digest {
        self.planning_context_digest
    }

    /// Returns the complete receipt identity.
    #[must_use]
    pub const fn receipt_digest(self) -> Digest {
        self.receipt_digest
    }
}

fn validate_request(request: &CandidateTransitionRequest) -> Result<(), TuneError> {
    if request.schema_version != CANDIDATE_TRANSITION_RECEIPT_SCHEMA_VERSION {
        return Err(receipt_mismatch(
            "the transition request schema is not supported",
        ));
    }
    validate_request_identities(
        request.session_digest,
        request.source_candidate_digest,
        request.target_candidate_digest,
        &request.validator,
        request.adjacency_policy_digest,
        request.planning_context_digest,
    )?;
    request.source.validate()?;
    request.target.validate()?;
    validate_candidate_pair(&request.source, &request.target)?;
    validate_candidate_digest(&request.source, request.source_candidate_digest, "source")?;
    validate_candidate_digest(&request.target, request.target_candidate_digest, "target")
}

fn validate_request_identities(
    session_digest: Digest,
    source_candidate_digest: Digest,
    target_candidate_digest: Digest,
    validator: &ArtifactIdentity,
    adjacency_policy_digest: Digest,
    planning_context_digest: Digest,
) -> Result<(), TuneError> {
    validator.validate()?;
    if session_digest.is_zero()
        || source_candidate_digest.is_zero()
        || target_candidate_digest.is_zero()
        || source_candidate_digest == target_candidate_digest
        || adjacency_policy_digest.is_zero()
        || planning_context_digest.is_zero()
    {
        return Err(TuneError::InvalidIdentity {
            detail: "the candidate transition request is incomplete".to_owned(),
        });
    }
    Ok(())
}

fn validate_candidate_pair(source: &Candidate, target: &Candidate) -> Result<(), TuneError> {
    if source == target {
        return Err(invalid_candidate("the candidate transition is unchanged"));
    }
    if source.lineage() != target.lineage() {
        return Err(invalid_candidate(
            "the candidate transition changed candidate lineage",
        ));
    }
    if source.parameters().keys().ne(target.parameters().keys()) {
        return Err(invalid_candidate(
            "the candidate transition changed the parameter set",
        ));
    }
    Ok(())
}

fn invalid_candidate(detail: impl Into<String>) -> TuneError {
    TuneError::InvalidCandidate {
        detail: detail.into(),
    }
}

fn receipt_mismatch(detail: impl Into<String>) -> TuneError {
    TuneError::ReceiptMismatch {
        operation: "authorize candidate transition",
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests;
