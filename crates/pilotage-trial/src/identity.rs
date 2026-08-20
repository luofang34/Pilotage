//! Immutable identity data for one trial run.

use serde::{Deserialize, Serialize};

use crate::{
    Digest, MAX_CLOCK_MAPPINGS, MAX_TEXT_BYTES, ValidationError,
    validation::{digest, nonempty_count, text},
};

/// The identity of one immutable artifact.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactIdentity {
    /// The stable artifact identifier.
    pub id: String,
    /// The artifact revision.
    pub revision: String,
    /// The artifact content digest.
    pub digest: Digest,
}

impl ArtifactIdentity {
    pub(crate) fn validate(&self, field: &str) -> Result<(), ValidationError> {
        text(&format!("{field}.id"), &self.id, MAX_TEXT_BYTES)?;
        text(&format!("{field}.revision"), &self.revision, MAX_TEXT_BYTES)?;
        digest(&format!("{field}.digest"), self.digest)
    }
}

/// The identity of one scenario revision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioIdentity {
    /// The stable scenario identifier.
    pub id: String,
    /// The scenario revision number.
    pub revision: u32,
    /// The digest of the canonical scenario JSON.
    pub digest: Digest,
}

impl ScenarioIdentity {
    pub(crate) fn validate(&self) -> Result<(), ValidationError> {
        text("run.scenario.id", &self.id, MAX_TEXT_BYTES)?;
        digest("run.scenario.digest", self.digest)
    }
}

/// A clock domain in a trial trace.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockDomain {
    /// The input device clock.
    Device,
    /// The control client clock.
    Client,
    /// The recorder host clock.
    Recorder,
    /// The adapter clock.
    Adapter,
    /// The flight controller clock.
    FlightController,
    /// The simulator clock.
    Simulator,
}

/// The quality of one clock mapping.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockMappingQuality {
    /// The source supplies an exact mapping.
    Exact,
    /// Measurement supplies an estimated mapping.
    Estimated,
    /// The mapping is not usable for timing analysis.
    Unusable,
}

/// A mapping between two nanosecond clocks.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockMapping {
    /// The source clock.
    pub from: ClockDomain,
    /// The target clock.
    pub to: ClockDomain,
    /// The target time minus the source time.
    pub offset_ns: i64,
    /// The maximum mapping uncertainty.
    pub uncertainty_ns: u64,
    /// The mapping quality.
    pub quality: ClockMappingQuality,
}

/// The immutable identity of one trial run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunIdentity {
    /// The unique run identifier.
    pub run_id: String,
    /// The code build identity.
    pub code_build: ArtifactIdentity,
    /// The vehicle adapter identity.
    pub vehicle_adapter: ArtifactIdentity,
    /// The vehicle adapter capabilities digest.
    pub adapter_capabilities_digest: Digest,
    /// The simulator backend capabilities digest.
    pub backend_capabilities_digest: Digest,
    /// The input device profile identity.
    pub device_profile: ArtifactIdentity,
    /// The control scheme identity.
    pub control_scheme: ArtifactIdentity,
    /// The control feel candidate identity.
    pub control_feel_candidate: ArtifactIdentity,
    /// The flight controller candidate identity.
    pub flight_controller_candidate: ArtifactIdentity,
    /// The simulator backend identity.
    pub simulator_backend: ArtifactIdentity,
    /// The simulator build identity.
    pub simulator: ArtifactIdentity,
    /// The simulated vehicle model identity.
    pub vehicle_model: ArtifactIdentity,
    /// The environmental condition set identity.
    pub condition_set: ArtifactIdentity,
    /// The scenario identity.
    pub scenario: ScenarioIdentity,
    /// The deterministic scenario seed.
    pub seed: u64,
    /// The repetition number for this run.
    pub repetition: u32,
    /// The mappings between clocks in this run.
    pub clock_mappings: Vec<ClockMapping>,
}

impl RunIdentity {
    /// Validates all required identity data.
    pub fn validate(&self) -> Result<(), ValidationError> {
        text("run.run_id", &self.run_id, MAX_TEXT_BYTES)?;
        self.validate_artifacts()?;
        digest(
            "run.adapter_capabilities_digest",
            self.adapter_capabilities_digest,
        )?;
        digest(
            "run.backend_capabilities_digest",
            self.backend_capabilities_digest,
        )?;
        self.scenario.validate()?;
        self.validate_clock_mappings()
    }

    fn validate_artifacts(&self) -> Result<(), ValidationError> {
        self.code_build.validate("run.code_build")?;
        self.vehicle_adapter.validate("run.vehicle_adapter")?;
        self.device_profile.validate("run.device_profile")?;
        self.control_scheme.validate("run.control_scheme")?;
        self.control_feel_candidate
            .validate("run.control_feel_candidate")?;
        self.flight_controller_candidate
            .validate("run.flight_controller_candidate")?;
        self.simulator_backend.validate("run.simulator_backend")?;
        self.simulator.validate("run.simulator")?;
        self.vehicle_model.validate("run.vehicle_model")?;
        self.condition_set.validate("run.condition_set")
    }

    fn validate_clock_mappings(&self) -> Result<(), ValidationError> {
        nonempty_count(
            "run.clock_mappings",
            self.clock_mappings.len(),
            MAX_CLOCK_MAPPINGS,
        )?;
        for (index, mapping) in self.clock_mappings.iter().enumerate() {
            if mapping.from == mapping.to {
                return Err(ValidationError::InvalidClockMapping {
                    index,
                    reason: "the source and target clocks are equal",
                });
            }
            self.check_mapping_is_unique(index, mapping)?;
        }
        Ok(())
    }

    fn check_mapping_is_unique(
        &self,
        index: usize,
        mapping: &ClockMapping,
    ) -> Result<(), ValidationError> {
        let duplicate = self.clock_mappings[..index]
            .iter()
            .any(|item| item.from == mapping.from && item.to == mapping.to);
        if duplicate {
            return Err(ValidationError::InvalidClockMapping {
                index,
                reason: "the clock pair occurs more than once",
            });
        }
        Ok(())
    }
}
