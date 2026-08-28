//! Canonical mission document contracts.

mod action;
mod canonical;
mod condition;
mod digest;
mod document;
mod engine;
mod error;
mod identity;
mod signal;
mod trial;
mod validation;

pub use action::{FlightAction, MissionAction, TransportLane, TrialAction};
pub use condition::{
    Comparison, MissionCondition, NavigationCondition, SignalCondition, SimulatorCondition,
    VehicleCondition, VehicleLifecycleState,
};
pub use digest::Digest;
pub use document::{
    ExecutionPolicy, ExecutionTarget, MissionCapability, MissionDocument, MissionPhase,
};
pub use engine::{
    AbortCause, ActionId, CleanupFailure, CleanupFailureKind, DeadlineClass, DirectiveContext,
    DirectivePurpose, DirectiveReceipt, EngineEvent, EngineInputError, EngineStart,
    EngineStartError, EngineState, FlightDirective, MissionDirective, MissionEngine,
    MissionObservation, MissionTerminal, NavigationObservation, ObservedSignal, PhaseStage,
    ReceiptResult, TickInput, TickOutput, TrialDirective, VehicleObservation, WallDeadline,
};
pub use error::{CodecError, ValidationError};
pub use identity::{
    ArtifactIdentity, FlightPlanReference, MissionIdentity, NavigationDataIdentity,
};
pub use signal::{
    ControlChannel, ControlValueField, QuaternionComponent, ReferenceFrame, SignalSelector,
    VectorComponent,
};
pub use trial::{
    ControlFamily, PhysicalUnit, ReferenceRule, SineComponent, StartHeading, StartState,
    StimulusEnvelope, StimulusError, StimulusMapping, Waveform,
};

/// The mission document schema version supported by this crate.
pub const MISSION_SCHEMA_VERSION: u16 = 3;

/// The maximum encoded mission document size.
pub const MAX_DOCUMENT_BYTES: usize = 1_048_576;

/// The maximum number of phases in one mission document.
pub const MAX_PHASES: usize = 1_024;

/// The maximum number of conditions in one phase condition list.
pub const MAX_PHASE_CONDITIONS: usize = 64;

/// The maximum number of capabilities in one phase.
pub const MAX_CAPABILITIES: usize = 32;

/// The maximum number of cleanup actions in one phase.
pub const MAX_CLEANUP_ACTIONS: usize = 32;

/// The maximum number of bytes in one text field.
pub const MAX_TEXT_BYTES: usize = 256;

#[cfg(test)]
mod tests;
