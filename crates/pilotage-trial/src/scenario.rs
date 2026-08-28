//! Versioned scenario and backend contracts.

mod backend;
mod phase;
mod selector;
mod stimulus;
mod waveform;

use serde::{Deserialize, Serialize};

use crate::{
    CodecError, Digest, MAX_MANIFEST_BYTES, MAX_PHASES, MAX_TEXT_BYTES, SCENARIO_SCHEMA_VERSION,
    ValidationError, canonical,
    validation::{nonempty_count, schema, text},
};

pub use backend::{BackendCapabilities, BackendCapability};
pub use phase::{Comparison, Phase, PhaseAction, PhaseCondition, StartHeading, StartState};
pub use selector::{
    ControlChannel, ControlValueField, QuaternionComponent, SignalSelectionError, SignalSelector,
    VectorComponent,
};
pub use stimulus::{
    ControlFamily, PhysicalUnit, ReferenceRule, StimulusEnvelope, StimulusError, StimulusMapping,
};
pub use waveform::{SineComponent, Waveform};

/// A versioned sequence of test phases.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// The scenario schema version.
    pub schema_version: u16,
    /// The stable scenario identifier.
    pub id: String,
    /// The scenario revision number.
    pub revision: u32,
    /// The ordered test phases.
    pub phases: Vec<Phase>,
}

impl Scenario {
    /// Decodes and validates a scenario JSON document.
    pub fn from_json(bytes: &[u8]) -> Result<Self, CodecError> {
        let value: Self = canonical::decode("scenario", bytes, MAX_MANIFEST_BYTES)?;
        value.validate()?;
        Ok(value)
    }

    /// Validates the scenario without a backend selection.
    pub fn validate(&self) -> Result<(), ValidationError> {
        schema("scenario", self.schema_version, SCENARIO_SCHEMA_VERSION)?;
        text("scenario.id", &self.id, MAX_TEXT_BYTES)?;
        nonempty_count("scenario.phases", self.phases.len(), MAX_PHASES)?;
        for (index, phase) in self.phases.iter().enumerate() {
            phase.validate(index)?;
            if self.phases[..index]
                .iter()
                .any(|prior| prior.id == phase.id)
            {
                return Err(ValidationError::DuplicateItem {
                    field: "scenario.phases.id".to_owned(),
                    index,
                });
            }
        }
        Ok(())
    }

    /// Validates the scenario against backend capabilities.
    pub fn validate_for_backend(
        &self,
        backend: &BackendCapabilities,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        backend.validate()?;
        for phase in &self.phases {
            for capability in &phase.required_capabilities {
                if !backend.supports(*capability) {
                    return Err(ValidationError::UnsupportedCapability {
                        phase: phase.id.clone(),
                        capability: capability.as_str().to_owned(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Encodes canonical compact JSON after validation.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, CodecError> {
        self.validate()?;
        canonical::encode("scenario", self, MAX_MANIFEST_BYTES)
    }

    /// Calculates the digest of canonical compact JSON.
    pub fn canonical_digest(&self) -> Result<Digest, CodecError> {
        self.to_canonical_json()
            .map(|bytes| canonical::digest(&bytes))
    }
}

#[cfg(test)]
mod tests;
