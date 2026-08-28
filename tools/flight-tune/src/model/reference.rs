use pilotage_mission_core::{MISSION_SCHEMA_VERSION, MissionDocument};
use pilotage_trial::Digest;
use serde::{Deserialize, Serialize};

use crate::TuneError;

#[cfg(test)]
#[path = "reference/tests.rs"]
mod tests;

const MAX_REVISION_ID_BYTES: usize = 128;
const MAX_SAMPLE_TIMEOUT_NS: u64 = 60_000_000_000;

/// A campaign reference to one canonical mission document.
///
/// The reference names the executed document by its identity. It also carries
/// the run limits that the campaign schedule applies. The sample timeout
/// repeats the document receipt timeout, so a stage cannot supply a limit that
/// the mission content digest does not cover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionReference {
    /// The immutable mission revision identifier.
    pub revision_id: String,
    /// The mission document schema version.
    pub schema_version: u16,
    /// The digest of the canonical mission content.
    pub content_digest: Digest,
    /// The largest permitted sample count for one run.
    pub max_samples: u32,
    /// The receipt timeout for each requested sample.
    pub sample_timeout_ns: u64,
}

impl MissionReference {
    /// Creates one reference to a stored mission document.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the document identity or a run limit is not
    /// valid.
    pub fn from_document(document: &MissionDocument, max_samples: u32) -> Result<Self, TuneError> {
        let reference = Self {
            revision_id: document.identity.revision_id.clone(),
            schema_version: document.identity.schema_version,
            content_digest: from_mission_digest(document.identity.content_digest),
            max_samples,
            sample_timeout_ns: document.execution_policy.receipt_timeout_ns,
        };
        reference.validate()?;
        Ok(reference)
    }

    /// Checks that one resolved document is the referenced mission.
    ///
    /// The check recalculates the content digest. A backend cannot present a
    /// document whose declared identity differs from its own bytes.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when an identity field or a run limit differs.
    pub fn verify_document(&self, document: &MissionDocument) -> Result<(), TuneError> {
        self.validate()?;
        let calculated = document
            .calculate_content_digest()
            .map_err(|source| mismatch(format!("cannot digest the resolved mission: {source}")))?;
        if document.identity.revision_id != self.revision_id {
            return Err(mismatch("the resolved mission revision differs"));
        }
        if document.identity.schema_version != self.schema_version {
            return Err(mismatch("the resolved mission schema version differs"));
        }
        if from_mission_digest(document.identity.content_digest) != self.content_digest
            || from_mission_digest(calculated) != self.content_digest
        {
            return Err(mismatch("the resolved mission content differs"));
        }
        if document.execution_policy.receipt_timeout_ns != self.sample_timeout_ns {
            return Err(mismatch("the resolved mission receipt timeout differs"));
        }
        Ok(())
    }

    /// Returns the wall-clock budget for one complete run.
    #[must_use]
    pub fn run_duration_ns(&self) -> u64 {
        self.sample_timeout_ns
            .saturating_mul(u64::from(self.max_samples))
            .max(1)
    }

    pub(crate) fn validate(&self) -> Result<(), TuneError> {
        if self.revision_id.trim().is_empty() || self.revision_id.len() > MAX_REVISION_ID_BYTES {
            return Err(invalid(format!(
                "a mission revision id needs 1 to {MAX_REVISION_ID_BYTES} bytes"
            )));
        }
        if self.schema_version != MISSION_SCHEMA_VERSION {
            return Err(invalid("the mission schema version is not supported"));
        }
        if self.content_digest.is_zero() {
            return Err(invalid("the mission content digest is zero"));
        }
        if self.max_samples == 0 {
            return Err(invalid("the mission sample ceiling is zero"));
        }
        if self.sample_timeout_ns == 0 || self.sample_timeout_ns > MAX_SAMPLE_TIMEOUT_NS {
            return Err(invalid("the mission sample timeout is out of range"));
        }
        Ok(())
    }
}

fn from_mission_digest(digest: pilotage_mission_core::Digest) -> Digest {
    Digest::from_bytes(*digest.as_bytes())
}

fn invalid(detail: impl Into<String>) -> TuneError {
    TuneError::InvalidStage {
        detail: detail.into(),
    }
}

fn mismatch(detail: impl Into<String>) -> TuneError {
    TuneError::ReceiptMismatch {
        operation: "resolve mission document",
        detail: detail.into(),
    }
}
