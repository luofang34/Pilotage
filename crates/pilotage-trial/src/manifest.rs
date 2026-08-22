//! The complete immutable contract for one trial run.

use serde::{Deserialize, Serialize};

use crate::{
    BackendCapabilities, CodecError, Digest, MAX_MANIFEST_BYTES, Scenario,
    TRIAL_MANIFEST_SCHEMA_VERSION, TrialSample, ValidationError, canonical, validation::schema,
};

/// The versioned immutable contract for one trial run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialManifest {
    /// The trial manifest schema version.
    pub schema_version: u16,
    /// The immutable run identity.
    pub run: crate::RunIdentity,
    /// The backend capability declaration.
    pub backend: BackendCapabilities,
    /// The scenario for this run.
    pub scenario: Scenario,
}

impl TrialManifest {
    /// Decodes and validates a trial manifest JSON document.
    pub fn from_json(bytes: &[u8]) -> Result<Self, CodecError> {
        let value: Self = canonical::decode("trial manifest", bytes, MAX_MANIFEST_BYTES)?;
        value.validate()?;
        Ok(value)
    }

    /// Validates the manifest and all identity links.
    pub fn validate(&self) -> Result<(), CodecError> {
        schema(
            "trial manifest",
            self.schema_version,
            TRIAL_MANIFEST_SCHEMA_VERSION,
        )?;
        self.run.validate()?;
        self.scenario.validate_for_backend(&self.backend)?;
        self.validate_identity_links()?;
        self.validate_condition_links()?;
        Ok(())
    }

    /// Encodes canonical compact JSON after validation.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, CodecError> {
        self.validate()?;
        canonical::encode("trial manifest", self, MAX_MANIFEST_BYTES)
    }

    /// Calculates the canonical trial manifest digest.
    pub fn canonical_digest(&self) -> Result<Digest, CodecError> {
        self.to_canonical_json()
            .map(|bytes| canonical::digest(&bytes))
    }

    /// Validates one sample and its relation to the prior sample.
    pub fn validate_sample(
        &self,
        previous: Option<&TrialSample>,
        sample: &TrialSample,
    ) -> Result<(), CodecError> {
        self.validate()?;
        self.validate_phase_index(sample)?;
        if let Some(previous) = previous {
            self.validate_phase_index(previous)?;
            sample.validate_after(previous, &self.run)?;
        } else {
            sample.validate_for_run(&self.run)?;
        }
        Ok(())
    }

    fn validate_identity_links(&self) -> Result<(), CodecError> {
        if self.run.simulator_backend != self.backend.backend {
            return Err(identity_mismatch("run.simulator_backend").into());
        }
        if self.run.backend_capabilities_digest != self.backend.canonical_digest()? {
            return Err(identity_mismatch("run.backend_capabilities_digest").into());
        }
        if self.run.scenario.id != self.scenario.id {
            return Err(identity_mismatch("run.scenario.id").into());
        }
        if self.run.scenario.revision != self.scenario.revision {
            return Err(identity_mismatch("run.scenario.revision").into());
        }
        if self.run.scenario.digest != self.scenario.canonical_digest()? {
            return Err(identity_mismatch("run.scenario.digest").into());
        }
        Ok(())
    }

    fn validate_condition_links(&self) -> Result<(), ValidationError> {
        for phase in &self.scenario.phases {
            if let crate::PhaseAction::ApplyConditions { condition_set } = &phase.action
                && condition_set != &self.run.condition_set
            {
                return Err(identity_mismatch("scenario.phase.condition_set"));
            }
        }
        Ok(())
    }

    fn validate_phase_index(&self, sample: &TrialSample) -> Result<(), ValidationError> {
        if usize::from(sample.phase_index) < self.scenario.phases.len() {
            return Ok(());
        }
        Err(ValidationError::PhaseOutOfRange {
            index: sample.phase_index,
            phase_count: self.scenario.phases.len(),
        })
    }
}

fn identity_mismatch(field: &str) -> ValidationError {
    ValidationError::IdentityMismatch {
        field: field.to_owned(),
    }
}

#[cfg(test)]
mod tests;
