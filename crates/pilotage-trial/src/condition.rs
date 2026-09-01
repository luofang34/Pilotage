//! Deterministic condition contracts for one trial run.
//!
//! A condition set holds one canonical executable value for each uncertainty
//! factor. Every field enters the canonical digest, so a changed request is a
//! changed condition identity. A perturbation stays constant in policy for a
//! complete run and decides from simulation sample identity, never wall time.

use serde::{Deserialize, Serialize};

mod actuator;
mod controller_initialization;
mod plant;
mod sensor;
mod timing;
mod wind;

pub use actuator::{
    ActuatorCondition, CommandHoldAction, CommandHoldIntervalIdentity, CommandLossPolicy,
};
pub use controller_initialization::{
    ControllerInitializationCondition, HoverThrustForceInitialization,
};
pub use plant::{HoverThrustExpectation, PlantCondition};
pub use sensor::{
    SensorAxis, SensorCondition, SensorNoiseLane, SensorNoiseReference, SensorReferenceLane,
};
pub use timing::{DelayJitter, TimingCondition};
pub use wind::{AppliedWind, GustEvent, HorizontalWind, TurbulenceModel, WindCondition};

use crate::{
    BackendCapabilities, BackendCapability, CONDITION_SET_SCHEMA_VERSION, CodecError, Digest,
    HoverEstimatorMode, MAX_MANIFEST_BYTES, MAX_TEXT_BYTES, ValidationError, canonical,
    validation::{schema, text},
};

/// A versioned deterministic condition artifact.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConditionSet {
    /// Condition schema version.
    pub schema_version: u16,
    /// Stable condition-set name.
    pub id: String,
    /// Condition-set revision.
    pub revision: u32,
    /// Seed for all deterministic disturbance components.
    pub seed: u64,
    /// Wind and turbulence definition.
    pub wind: WindCondition,
    /// Deterministic source timing perturbation.
    pub timing: TimingCondition,
    /// Deterministic flight-controller sensor perturbations.
    pub sensor: SensorCondition,
    /// Deterministic actuator perturbations.
    pub actuator: ActuatorCondition,
    /// Controller values that change before controller construction.
    pub controller_initialization: ControllerInitializationCondition,
    /// Simulator plant variation.
    pub plant: PlantCondition,
}

impl ConditionSet {
    /// Decode and validate a condition-set JSON document.
    ///
    /// # Errors
    ///
    /// Returns an error when the document is too large, holds an unknown
    /// field, or fails the condition contract.
    pub fn from_json(bytes: &[u8]) -> Result<Self, CodecError> {
        let value: Self = canonical::decode("condition set", bytes, MAX_MANIFEST_BYTES)?;
        value.validate()?;
        Ok(value)
    }

    /// Validate the condition-set contract.
    ///
    /// # Errors
    ///
    /// Returns an error when any requested value is outside its fixed bound.
    pub fn validate(&self) -> Result<(), ValidationError> {
        schema(
            "condition set",
            self.schema_version,
            CONDITION_SET_SCHEMA_VERSION,
        )?;
        text("condition_set.id", &self.id, MAX_TEXT_BYTES)?;
        self.wind.validate()?;
        self.timing.validate()?;
        self.sensor.validate()?;
        self.actuator.validate()?;
        self.controller_initialization.validate()?;
        self.plant.validate()
    }

    /// Returns the exact capabilities that this condition set needs.
    ///
    /// A nominal factor needs no capability, so a nominal condition keeps the
    /// current backend requirements.
    #[must_use]
    pub fn required_capabilities(&self) -> Vec<BackendCapability> {
        let mut required = Vec::new();
        if !self.sensor.is_nominal() {
            required.push(BackendCapability::SensorPerturbation);
        }
        required.extend(self.actuator.required_capabilities());
        required.extend(self.controller_initialization.required_capabilities());
        required
    }

    /// Validates this condition against one complete backend declaration.
    ///
    /// # Errors
    ///
    /// Returns an error when the condition or the declaration is invalid, or
    /// when the backend does not report an exact required capability.
    pub fn validate_for_backend(
        &self,
        backend: &BackendCapabilities,
    ) -> Result<(), ValidationError> {
        backend.validate()?;
        self.validate_capability_report(&backend.capabilities, backend.hover_estimator_mode)
    }

    /// Validates this condition against a typed capability report.
    ///
    /// Preparation calls this with the known backend declaration. Arming
    /// calls it again with the live report, so a runtime that changed its
    /// declaration cannot reach a run.
    ///
    /// # Errors
    ///
    /// Returns an error when a required capability is absent, or when a
    /// non-nominal hover force meets an active hover estimator.
    pub fn validate_capability_report(
        &self,
        capabilities: &[BackendCapability],
        hover_estimator_mode: HoverEstimatorMode,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        for capability in self.required_capabilities() {
            if !capabilities.contains(&capability) {
                return Err(ValidationError::UnsupportedConditionCapability {
                    capability: capability.as_str().to_owned(),
                });
            }
        }
        if !self
            .controller_initialization
            .has_nominal_hover_thrust_force()
            && !hover_estimator_mode.is_inactive()
        {
            return Err(ValidationError::ActiveHoverEstimator {
                mode: hover_estimator_mode.as_str().to_owned(),
            });
        }
        Ok(())
    }

    /// Builds the stable identity for one complete command-hold interval.
    ///
    /// # Errors
    ///
    /// Returns an error when the condition cannot produce a canonical digest.
    pub fn command_hold_interval_identity(
        &self,
        run_seed: u64,
        interval_epoch: u64,
        interval_index: u64,
        first_eligible_global_sample_sequence: u64,
    ) -> Result<CommandHoldIntervalIdentity, CodecError> {
        CommandHoldIntervalIdentity::new(
            self.canonical_digest()?,
            run_seed,
            interval_epoch,
            interval_index,
            first_eligible_global_sample_sequence,
        )
        .map_err(CodecError::from)
    }

    /// Builds the reference command-hold decisions for one interval.
    ///
    /// # Errors
    ///
    /// Returns an error when the condition or the command-loss policy is
    /// invalid.
    pub fn command_hold_decisions_for_interval(
        &self,
        run_seed: u64,
        interval_epoch: u64,
        interval_index: u64,
        first_eligible_global_sample_sequence: u64,
    ) -> Result<Vec<bool>, CodecError> {
        let identity = self.command_hold_interval_identity(
            run_seed,
            interval_epoch,
            interval_index,
            first_eligible_global_sample_sequence,
        )?;
        self.actuator
            .command_loss
            .decisions_for_interval(identity)
            .map_err(CodecError::from)
    }

    /// Builds the reference sensor perturbations for one global sample.
    ///
    /// # Errors
    ///
    /// Returns an error when the condition cannot produce a canonical digest.
    pub fn sensor_references_for_sample(
        &self,
        run_seed: u64,
        global_sample_sequence: u64,
    ) -> Result<Vec<SensorNoiseReference>, CodecError> {
        let condition_digest = self.canonical_digest()?;
        Ok(self
            .sensor
            .noise_lanes()
            .iter()
            .copied()
            .map(|lane| {
                SensorNoiseReference::new(condition_digest, run_seed, global_sample_sequence, lane)
            })
            .collect())
    }

    /// Encode canonical compact JSON after validation.
    ///
    /// # Errors
    ///
    /// Returns an error when the condition is invalid or too large.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, CodecError> {
        self.validate()?;
        canonical::encode("condition set", self, MAX_MANIFEST_BYTES)
    }

    /// Calculate the canonical condition-set identity.
    ///
    /// # Errors
    ///
    /// Returns an error when the condition cannot be encoded.
    pub fn canonical_digest(&self) -> Result<Digest, CodecError> {
        self.to_canonical_json()
            .map(|bytes| canonical::digest(&bytes))
    }

    /// Resolve the wind request at one simulator-time offset.
    ///
    /// # Errors
    ///
    /// Returns an error when the condition is invalid.
    pub fn wind_at(&self, elapsed_ns: u64) -> Result<AppliedWind, ValidationError> {
        self.wind_at_for_run(0, elapsed_ns)
    }

    /// Resolve the wind for one recorded run seed and simulator-time offset.
    ///
    /// The artifact seed defines the condition. The run seed gives each
    /// repetition a separate deterministic disturbance without changing the
    /// condition artifact identity.
    ///
    /// # Errors
    ///
    /// Returns an error when the condition is invalid.
    pub fn wind_at_for_run(
        &self,
        run_seed: u64,
        elapsed_ns: u64,
    ) -> Result<AppliedWind, ValidationError> {
        self.validate()?;
        Ok(self
            .wind
            .resolve(self.seed ^ run_seed.rotate_left(17), elapsed_ns))
    }

    /// Resolve the requested vehicle-source delay for one run and time.
    ///
    /// # Errors
    ///
    /// Returns an error when the condition is invalid.
    pub fn source_delay_ns_for_run(
        &self,
        run_seed: u64,
        elapsed_ns: u64,
    ) -> Result<u64, ValidationError> {
        self.validate()?;
        Ok(self.timing.delay_ns(self.seed, run_seed, elapsed_ns))
    }
}

#[cfg(test)]
mod tests;
