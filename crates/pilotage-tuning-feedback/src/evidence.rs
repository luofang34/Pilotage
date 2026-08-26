use std::path::{Path, PathBuf};

use flight_tune::{Digest, FinalQualificationOutcome, JournalEvidenceSnapshot};
use serde::{Deserialize, Serialize};

use crate::{FeedbackError, digest, qualification, storage};

/// The supported campaign evidence schema.
pub const CAMPAIGN_EVIDENCE_SCHEMA_VERSION: u16 = 1;

/// One simulator-neutral tuning campaign evidence document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignEvidence {
    pub(crate) schema_version: u16,
    pub(crate) journal: JournalEvidenceSnapshot,
}

/// The immutable location and identity of one evidence document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceReceipt {
    /// The SHA-256 identity of the exact JSON bytes.
    pub digest: Digest,
    /// The content-addressed evidence object path.
    pub object_path: PathBuf,
}

/// Campaign evidence that passed independent verification.
#[derive(Debug, Clone)]
pub struct VerifiedCampaignEvidence {
    evidence: CampaignEvidence,
    source_digest: Digest,
}

/// Sealed campaign evidence with an independently verified qualified result.
#[derive(Debug, Clone)]
pub struct VerifiedQualifiedEvidence {
    campaign: VerifiedCampaignEvidence,
    selected_candidate: Digest,
}

impl CampaignEvidence {
    /// Creates and verifies evidence from one authenticated journal snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`FeedbackError`] when any identity or result is not reproducible.
    pub fn new(journal: JournalEvidenceSnapshot) -> Result<Self, FeedbackError> {
        let evidence = Self {
            schema_version: CAMPAIGN_EVIDENCE_SCHEMA_VERSION,
            journal,
        };
        qualification::verify(&evidence)?;
        Ok(evidence)
    }

    /// Returns the exact journal evidence payload.
    #[must_use]
    pub const fn journal(&self) -> &JournalEvidenceSnapshot {
        &self.journal
    }

    /// Stores canonical JSON as one immutable content-addressed object.
    ///
    /// # Errors
    ///
    /// Returns [`FeedbackError`] when verification or durable storage fails.
    pub fn store_content_addressed_blocking(
        &self,
        root: impl AsRef<Path>,
    ) -> Result<EvidenceReceipt, FeedbackError> {
        qualification::verify(self)?;
        storage::store_blocking(self, root.as_ref())
    }
}

impl VerifiedCampaignEvidence {
    /// Decodes canonical JSON and independently verifies all evidence.
    ///
    /// # Errors
    ///
    /// Returns [`FeedbackError`] when decoding, canonical form, or verification fails.
    pub fn from_bytes(bytes: &[u8], expected_digest: Digest) -> Result<Self, FeedbackError> {
        let source_digest = digest::hash(bytes);
        if expected_digest.is_zero() || source_digest != expected_digest {
            return Err(crate::error::invalid(
                "the campaign evidence transport digest changed",
            ));
        }
        let evidence: CampaignEvidence =
            serde_json::from_slice(bytes).map_err(|source| FeedbackError::Decode {
                document: "campaign evidence",
                source,
            })?;
        if digest::encode("campaign evidence", &evidence)? != bytes {
            return Err(crate::error::invalid(
                "campaign evidence does not use canonical JSON bytes",
            ));
        }
        qualification::verify(&evidence)?;
        Ok(Self {
            evidence,
            source_digest,
        })
    }

    /// Loads one content-addressed evidence object and verifies exact readback.
    ///
    /// # Errors
    ///
    /// Returns [`FeedbackError`] when the file or evidence is not valid.
    pub fn load_content_addressed_blocking(path: impl AsRef<Path>) -> Result<Self, FeedbackError> {
        let bytes = storage::load_blocking(path.as_ref())?;
        let source_digest = digest::hash(&bytes);
        let verified = Self::from_bytes(&bytes, source_digest)?;
        storage::require_name(path.as_ref(), verified.source_digest)?;
        Ok(verified)
    }

    /// Returns the exact source JSON identity.
    #[must_use]
    pub const fn source_digest(&self) -> Digest {
        self.source_digest
    }

    /// Returns the candidate selected by the closed promotion round.
    #[must_use]
    pub const fn selected_candidate(&self) -> Option<Digest> {
        self.evidence.journal.promotion_closure.selected_candidate
    }

    /// Returns the sealed final result, when final qualification is complete.
    #[must_use]
    pub const fn outcome(&self) -> Option<&FinalQualificationOutcome> {
        self.evidence.journal.final_outcome.as_ref()
    }

    /// Requires an independently reproduced qualified final result.
    ///
    /// # Errors
    ///
    /// Returns [`FeedbackError`] unless a selected candidate passed final qualification.
    pub fn verify_qualified(self) -> Result<VerifiedQualifiedEvidence, FeedbackError> {
        let selected_candidate = qualification::verify_qualified(&self.evidence)?;
        Ok(VerifiedQualifiedEvidence {
            campaign: self,
            selected_candidate,
        })
    }

    /// Returns the verified campaign document.
    #[must_use]
    pub const fn evidence(&self) -> &CampaignEvidence {
        &self.evidence
    }
}

impl VerifiedQualifiedEvidence {
    /// Returns the candidate authorized by promotion and final qualification.
    #[must_use]
    pub const fn selected_candidate(&self) -> Digest {
        self.selected_candidate
    }

    /// Returns the complete verified campaign evidence.
    #[must_use]
    pub const fn campaign(&self) -> &VerifiedCampaignEvidence {
        &self.campaign
    }
}
