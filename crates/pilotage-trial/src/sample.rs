//! Versioned samples for one trial trace.

mod control;
mod missing;
mod state;
mod time;

use serde::{Deserialize, Serialize};

use crate::{
    CodecError, Digest, MAX_SAMPLE_BYTES, TRIAL_SAMPLE_SCHEMA_VERSION, ValidationError, canonical,
    validation::{digest, schema},
};

pub use control::{AdapterDisposition, ControlAxes, ControlValue, RawInput, ReferenceFrame};
pub use missing::{MissingReason, MissingSignal, Observed};
pub use state::{
    ActuatorState, ConditionState, HealthState, KinematicState, LifecycleObservation,
    LifecycleState, NamedValue, Quaternion, Vector3,
};
pub use time::SampleTime;

/// One versioned sample in a trial trace.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialSample {
    /// The trial sample schema version.
    pub schema_version: u16,
    /// The canonical digest of the trial manifest.
    pub run_digest: Digest,
    /// The monotonic sequence number in this run.
    pub sequence: u64,
    /// The number of lost samples before this sample.
    pub dropped_before: u32,
    /// The zero-based scenario phase index.
    pub phase_index: u16,
    /// The sample times in all available clocks.
    pub time: SampleTime,
    /// The raw device input.
    pub raw_input: Observed<RawInput>,
    /// The normalized input channels.
    pub normalized_control: Observed<ControlAxes>,
    /// The typed control intent.
    pub typed_intent: Observed<ControlValue>,
    /// The demand that the adapter receives.
    pub adapter_demand: Observed<ControlValue>,
    /// The setpoint that the adapter transmits.
    pub transmitted_setpoint: Observed<ControlValue>,
    /// The flight controller kinematic estimate.
    pub flight_controller_estimate: Observed<KinematicState>,
    /// The simulator kinematic truth evidence.
    pub simulator_truth: Observed<KinematicState>,
    /// The actuator effort and saturation state.
    pub actuator: Observed<ActuatorState>,
    /// The adapter decision for the control demand.
    pub adapter_disposition: Observed<AdapterDisposition>,
    /// The simulator and vehicle lifecycle state.
    pub lifecycle: Observed<LifecycleObservation>,
    /// The measured environmental condition state.
    pub condition_state: Observed<ConditionState>,
    /// The control link validity state.
    pub link_state: Observed<HealthState>,
    /// The flight controller estimator validity state.
    pub estimator_state: Observed<HealthState>,
}

impl TrialSample {
    /// Decodes and validates a trial sample JSON document.
    pub fn from_json(bytes: &[u8]) -> Result<Self, CodecError> {
        let value: Self = canonical::decode("trial sample", bytes, MAX_SAMPLE_BYTES)?;
        value.validate()?;
        Ok(value)
    }

    /// Validates the sample content and fixed collection limits.
    pub fn validate(&self) -> Result<(), ValidationError> {
        schema(
            "trial sample",
            self.schema_version,
            TRIAL_SAMPLE_SCHEMA_VERSION,
        )?;
        digest("sample.run_digest", self.run_digest)?;
        self.time.validate()?;
        self.raw_input
            .validate_with("sample.raw_input", RawInput::validate)?;
        self.normalized_control
            .validate_with("sample.normalized_control", ControlAxes::validate)?;
        self.typed_intent
            .validate_with("sample.typed_intent", ControlValue::validate)?;
        self.adapter_demand
            .validate_with("sample.adapter_demand", ControlValue::validate)?;
        self.transmitted_setpoint
            .validate_with("sample.transmitted_setpoint", ControlValue::validate)?;
        self.validate_state_signals()?;
        self.validate_status_signals()
    }

    /// Encodes canonical compact JSON after validation.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, CodecError> {
        self.validate()?;
        canonical::encode("trial sample", self, MAX_SAMPLE_BYTES)
    }

    /// Calculates the digest of canonical compact JSON.
    pub fn canonical_digest(&self) -> Result<Digest, CodecError> {
        self.to_canonical_json()
            .map(|bytes| canonical::digest(&bytes))
    }

    fn validate_state_signals(&self) -> Result<(), ValidationError> {
        self.flight_controller_estimate.validate_with(
            "sample.flight_controller_estimate",
            KinematicState::validate,
        )?;
        self.simulator_truth
            .validate_with("sample.simulator_truth", KinematicState::validate)?;
        self.actuator
            .validate_with("sample.actuator", ActuatorState::validate)?;
        self.condition_state
            .validate_with("sample.condition_state", ConditionState::validate)
    }

    fn validate_status_signals(&self) -> Result<(), ValidationError> {
        self.adapter_disposition
            .validate_with("sample.adapter_disposition", AdapterDisposition::validate)?;
        self.lifecycle
            .validate_with("sample.lifecycle", LifecycleObservation::validate)?;
        self.link_state
            .validate_with("sample.link_state", HealthState::validate)?;
        self.estimator_state
            .validate_with("sample.estimator_state", HealthState::validate)
    }
}
