use pilotage_trial::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as ShaDigest, Sha256};

use crate::TuneError;

/// The content identity of one runtime artifact or implementation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactIdentity {
    /// The stable artifact name.
    pub id: String,
    /// The SHA-256 digest of the exact artifact or configuration.
    pub digest: Digest,
}

impl ArtifactIdentity {
    /// Creates a validated artifact identity.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the name is empty or the digest is zero.
    pub fn new(id: impl Into<String>, digest: Digest) -> Result<Self, TuneError> {
        let identity = Self {
            id: id.into(),
            digest,
        };
        identity.validate()?;
        Ok(identity)
    }

    /// Creates an identity from exact text content.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the name is empty.
    pub fn from_text(id: impl Into<String>, text: &str) -> Result<Self, TuneError> {
        Self::new(id, digest_bytes(text.as_bytes()))
    }

    pub(crate) fn validate(&self) -> Result<(), TuneError> {
        if self.id.trim().is_empty() || self.id.len() > 256 || self.digest.is_zero() {
            return Err(TuneError::InvalidIdentity {
                detail: "an artifact identity needs a short name and a nonzero digest".to_owned(),
            });
        }
        Ok(())
    }
}

/// The immutable source identity for all candidates in one session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateLineage {
    /// The candidate document schema identity.
    pub schema: String,
    /// The digest of the immutable base preset.
    pub base_preset_digest: Digest,
    /// The digest of the plant identification artifact.
    pub plant_digest: Digest,
}

impl CandidateLineage {
    /// Validates the candidate lineage.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when one identity is missing.
    pub fn validate(&self) -> Result<(), TuneError> {
        if self.schema.trim().is_empty()
            || self.schema.len() > 128
            || self.base_preset_digest.is_zero()
            || self.plant_digest.is_zero()
        {
            return Err(TuneError::InvalidIdentity {
                detail: "candidate lineage is incomplete".to_owned(),
            });
        }
        Ok(())
    }
}

/// All executable and plant identities for one tuning session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeIdentities {
    /// The tuning harness build.
    pub harness_build: ArtifactIdentity,
    /// The proposal strategy and its exact configuration.
    pub strategy: ArtifactIdentity,
    /// The continuous metric implementation and its configuration.
    pub metric: ArtifactIdentity,
    /// The streaming hard gate implementation and its configuration.
    pub hard_gates: ArtifactIdentity,
    /// The simulator build and adapter configuration.
    pub simulator: ArtifactIdentity,
    /// The selected simulator airframe artifact.
    pub airframe: ArtifactIdentity,
    /// The vehicle controller build and adapter configuration.
    pub vehicle: ArtifactIdentity,
    /// The candidate-transition validator and its exact configuration.
    pub transition_validator: ArtifactIdentity,
    /// The exact vehicle adjacency-policy digest.
    pub adjacency_policy_digest: Digest,
}

impl RuntimeIdentities {
    pub(crate) fn validate(&self) -> Result<(), TuneError> {
        for identity in [
            &self.harness_build,
            &self.strategy,
            &self.metric,
            &self.hard_gates,
            &self.simulator,
            &self.airframe,
            &self.vehicle,
            &self.transition_validator,
        ] {
            identity.validate()?;
        }
        if self.adjacency_policy_digest.is_zero() {
            return Err(TuneError::InvalidIdentity {
                detail: "the vehicle adjacency-policy digest is zero".to_owned(),
            });
        }
        Ok(())
    }
}

pub(crate) fn harness_build_identity() -> ArtifactIdentity {
    ArtifactIdentity {
        id: "flight-tune-build".to_owned(),
        digest: digest_bytes(env!("FLIGHT_TUNE_BUILD_ID").as_bytes()),
    }
}

pub(crate) fn digest_bytes(bytes: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Digest::from_bytes(hasher.finalize().into())
}
