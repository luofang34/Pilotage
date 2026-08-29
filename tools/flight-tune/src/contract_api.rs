//! Public re-exports of the versioned contracts a campaign host shares.
//!
//! A backend, an adapter, and a vehicle action port all speak the mission and
//! trial contracts. Re-exporting them here gives every host one name for each
//! contract type and one crate to depend on.

pub use pilotage_mission_core::{
    ArtifactIdentity as MissionArtifactIdentity, ControlChannel, ControlFamily, ControlValueField,
    Digest as MissionDigest, DirectiveContext, ExecutionTarget, FlightAction,
    MISSION_SCHEMA_VERSION, MissionAction, MissionCapability, MissionDirective, MissionDocument,
    MissionTerminal, ObservedSignal, PhysicalUnit, ReceiptResult, ReferenceFrame, ReferenceRule,
    SignalSelector, SineComponent, StartHeading, StartState, StimulusEnvelope, StimulusError,
    StimulusMapping, TrialAction, VehicleLifecycleState, Waveform,
};
pub use pilotage_trial::{
    BackendCapability, CONDITION_SET_SCHEMA_VERSION, ConditionSet, Digest, HoverEstimatorMode,
    Scenario as TrialScenario,
};
