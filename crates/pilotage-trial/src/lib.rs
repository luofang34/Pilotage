//! Versioned data contracts for repeatable control trials.
//!
//! This crate contains data only. A runner supplies all input and output.

mod canonical;
mod digest;
mod error;
mod identity;
mod limits;
mod sample;
mod validation;

pub use digest::Digest;
pub use error::{CodecError, ValidationError};
pub use limits::{
    MAX_CONTROL_EVENT_HISTORY, MAX_RUN_IDENTITY_BYTES, QUATERNION_NORM_TOLERANCE,
    RUN_IDENTITY_SCHEMA_VERSION,
};
pub use sample::{
    CausalStage, ClockReading, ControlEventId, ControlStage, SimulatorTruthEvidence, SourceStamp,
    StageProducerRole, StageStamp, TrialStreamValidator,
};
pub use identity::{
    ArtifactIdentity, ClockDomain, ClockMapping, ClockMappingQuality, RunIdentity, ScenarioIdentity,
};
pub use identity::RecorderTimeInterval;
pub use limits::{
    BACKEND_CAPABILITIES_SCHEMA_VERSION, MAX_ACTUATOR_VALUES, MAX_CAPABILITIES, MAX_CLOCK_MAPPINGS,
    MAX_CONDITION_VALUES, MAX_MANIFEST_BYTES, MAX_PHASE_CONDITIONS, MAX_PHASES, MAX_RAW_AXES,
    MAX_RAW_BUTTONS, MAX_SAMPLE_BYTES, MAX_TEXT_BYTES, MAX_WAVE_COMPONENTS,
    SCENARIO_SCHEMA_VERSION, TRIAL_MANIFEST_SCHEMA_VERSION, TRIAL_SAMPLE_SCHEMA_VERSION,
};
pub use sample::{
    ActuatorState, AdapterDisposition, ConditionState, ControlAxes, ControlValue, HealthState,
    KinematicState, LifecycleObservation, LifecycleState, MissingReason, MissingSignal, NamedValue,
    Observed, Quaternion, RawInput, ReferenceFrame, SampleTime, TrialSample, Vector3,
};
