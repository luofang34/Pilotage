//! Sealing one run, and saying plainly when it cannot be sealed.
//!
//! A sealed run states four things together: which run intent it flew,
//! which runtime implementation flew it, what the run's direct evidence
//! was, and how it ended. Binding them in one document is what stops a
//! reader pairing evidence from one runtime identity with a result from
//! another.
//!
//! A run whose durable direct ledger has a prepared command with no result
//! is not sealed at all. It is quarantined, because nobody can say whether
//! the vehicle was commanded.

use flight_tune::{ArtifactIdentity, Digest, ScenarioStopContext, ScenarioStopReason};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::AviateRuntimeError;
use super::direct::DirectRunEvidence;

/// The supported Aviate run seal schema.
pub const RUN_SEAL_SCHEMA_VERSION: u16 = 1;

const RUN_SEAL_DOMAIN: &[u8] = b"pilotage-aviate-run-seal-v1\0";

/// How one Aviate run ended, as the vehicle port saw it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunEnding {
    /// The mission engine reached a terminal result.
    Mission,
    /// A streaming hard gate ended the run.
    HardGate,
    /// A sample request reached its finite timeout.
    SampleTimeout,
    /// An execution or evidence error ended the run.
    ExecutionError,
}

impl RunEnding {
    /// The vehicle-port ending for one neutral stop reason.
    #[must_use]
    pub const fn of(reason: &ScenarioStopReason) -> Self {
        match reason {
            ScenarioStopReason::Mission(_) => Self::Mission,
            ScenarioStopReason::HardGate => Self::HardGate,
            ScenarioStopReason::SampleTimeout => Self::SampleTimeout,
            ScenarioStopReason::ExecutionError => Self::ExecutionError,
        }
    }
}

/// Everything one sealed Aviate run is answerable for.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AviateRunSeal {
    /// Document schema version.
    pub schema_version: u16,
    /// The exact run intent this run flew.
    pub run_intent_digest: Digest,
    /// The runtime implementation that flew it.
    pub runtime_identity: ArtifactIdentity,
    /// How the run ended.
    pub ending: RunEnding,
    /// The last source sequence the vehicle port consumed.
    pub last_source_sequence: Option<u64>,
    /// The number of frames the vehicle port accepted.
    pub accepted_frames: u64,
    /// The identity of this run's direct evidence, when the run had any.
    pub direct_evidence_digest: Option<Digest>,
}

impl AviateRunSeal {
    /// The identity a terminal receipt binds this seal by.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the document cannot be encoded.
    pub fn digest(&self) -> Result<Digest, AviateRuntimeError> {
        let bytes = serde_json::to_vec(self).map_err(|source| AviateRuntimeError::Encode {
            document: "run seal",
            source,
        })?;
        let mut hasher = Sha256::new();
        hasher.update(RUN_SEAL_DOMAIN);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
        Ok(Digest::from_bytes(hasher.finalize().into()))
    }

    /// Rejects a seal that does not name one exact run and runtime.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the run intent or the runtime
    /// identity differs.
    pub fn require_bound(
        &self,
        run_intent_digest: Digest,
        runtime_identity: &ArtifactIdentity,
    ) -> Result<(), AviateRuntimeError> {
        if self.schema_version != RUN_SEAL_SCHEMA_VERSION
            || self.run_intent_digest != run_intent_digest
            || &self.runtime_identity != runtime_identity
        {
            return Err(AviateRuntimeError::UnboundRunSeal);
        }
        Ok(())
    }
}

/// What the vehicle port knows about one run when it stops.
#[derive(Clone, Debug, PartialEq)]
pub struct RunClosure {
    /// The exact run intent that stopped.
    pub run_intent_digest: Digest,
    /// The runtime implementation that flew it.
    pub runtime_identity: ArtifactIdentity,
    /// The number of frames the vehicle port accepted.
    pub accepted_frames: u64,
    /// The direct evidence the run collected, when it commanded directly.
    pub direct_evidence: Option<DirectRunEvidence>,
}

/// Seals one stopped run.
///
/// # Errors
///
/// Returns [`AviateRuntimeError`] when the direct evidence does not bind
/// this run, or when the seal cannot be encoded.
pub fn seal(
    closure: &RunClosure,
    context: &ScenarioStopContext,
) -> Result<AviateRunSeal, AviateRuntimeError> {
    let direct_evidence_digest = match &closure.direct_evidence {
        Some(evidence) => {
            evidence.require_bound(closure.run_intent_digest, &closure.runtime_identity)?;
            Some(evidence.digest()?)
        }
        None => None,
    };
    let seal = AviateRunSeal {
        schema_version: RUN_SEAL_SCHEMA_VERSION,
        run_intent_digest: closure.run_intent_digest,
        runtime_identity: closure.runtime_identity.clone(),
        ending: RunEnding::of(&context.reason),
        last_source_sequence: context.last_source_sequence,
        accepted_frames: closure.accepted_frames,
        direct_evidence_digest,
    };
    seal.require_bound(closure.run_intent_digest, &closure.runtime_identity)?;
    Ok(seal)
}
