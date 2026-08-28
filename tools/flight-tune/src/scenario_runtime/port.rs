use pilotage_mission_core::{MissionCapability, MissionDirective, MissionTerminal, ReceiptResult};
use thiserror::Error;

use crate::{ArtifactIdentity, RunExecutionContext, TuneError};

use super::ScenarioFrame;

/// One vehicle-runtime receipt for a neutral observation frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioObservationReceipt {
    /// The source sequence that the runtime consumed.
    pub source_sequence: u64,
    /// The action result, when one directive completed on this frame.
    pub action_result: Option<ReceiptResult>,
}

/// The terminal context shared by the campaign host and action port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioStopContext {
    /// The semantic reason that ends action execution.
    pub reason: ScenarioStopReason,
    /// The last source sequence consumed by the mission runtime.
    pub last_source_sequence: Option<u64>,
}

/// The semantic reason that ends one scenario action runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScenarioStopReason {
    /// The mission engine reached a terminal result.
    Mission(MissionTerminal),
    /// A streaming hard gate ended the run.
    HardGate,
    /// A sample request reached its finite timeout.
    SampleTimeout,
    /// An execution or evidence error ended the run.
    ExecutionError,
}

/// A neutral scenario-runtime operation failed.
#[derive(Debug, Error)]
pub enum ScenarioRuntimeError {
    /// The scenario document cannot be projected into a mission document.
    #[error("scenario document projection failed: {detail}")]
    DocumentProjection {
        /// The stable projection detail.
        detail: String,
    },
    /// A scenario runtime identity is incomplete.
    #[error("scenario runtime identity is invalid: {source}")]
    InvalidIdentity {
        /// The identity validation failure.
        #[source]
        source: crate::TuneError,
    },
    /// A frame is incomplete or inconsistent.
    #[error("scenario frame is invalid: {detail}")]
    InvalidFrame {
        /// The stable frame detail.
        detail: String,
    },
    /// The action port identity differs from the admitted runtime identity.
    #[error("the scenario action-port identity does not match the admitted runtime")]
    IdentityMismatch,
    /// The mission engine refused the start input.
    #[error("mission engine start failed: {source}")]
    EngineStart {
        /// The engine start error.
        #[source]
        source: pilotage_mission_core::EngineStartError,
    },
    /// The mission engine refused one tick input.
    #[error("mission engine input failed: {source}")]
    EngineInput {
        /// The engine input error.
        #[source]
        source: pilotage_mission_core::EngineInputError,
    },
    /// The admitted mission engine is absent when a tick starts.
    #[error("the admitted mission engine is absent")]
    EngineAbsent,
    /// The action port returned a receipt without an outstanding directive.
    #[error("the scenario action port returned an uncorrelated receipt")]
    UncorrelatedReceipt,
    /// The action port consumed the wrong source frame.
    #[error("the scenario action port receipt has source sequence {actual}; expected {expected}")]
    SourceSequenceMismatch {
        /// The expected source sequence.
        expected: u64,
        /// The reported source sequence.
        actual: u64,
    },
    /// The mission engine emitted more than one directive in one tick.
    #[error("the mission engine emitted {count} directives in one tick")]
    DirectiveCount {
        /// The emitted directive count.
        count: usize,
    },
    /// The active runtime does not supply a capability that a phase requires.
    #[error("scenario phase {phase_id} requires unsupported capability {capability:?}")]
    MissingCapability {
        /// The phase that declares the capability.
        phase_id: String,
        /// The unsupported capability.
        capability: MissionCapability,
    },
    /// Campaign authority changed before an action-port mutation.
    #[error("campaign authority check failed: {source}")]
    Authority {
        /// The journal authority failure.
        #[source]
        source: TuneError,
    },
    /// The action port failed.
    #[error("scenario action port failed during {operation}: {detail}")]
    ActionPort {
        /// The action-port operation.
        operation: &'static str,
        /// The stable action-port detail.
        detail: String,
    },
    /// An action-port operation and its required containment both failed.
    #[error(
        "scenario action port failed during {operation}: {primary}; containment failed: {containment}"
    )]
    ActionAndContainment {
        /// The operation that failed before containment.
        operation: &'static str,
        /// The primary action-port failure.
        primary: Box<ScenarioRuntimeError>,
        /// The stop or cleanup failure.
        #[source]
        containment: Box<ScenarioRuntimeError>,
    },
    /// Stop and cleanup both failed during containment.
    #[error("scenario action-port stop failed: {stop}; cleanup also failed: {cleanup}")]
    StopAndCleanup {
        /// The stop failure.
        stop: Box<ScenarioRuntimeError>,
        /// The cleanup failure.
        #[source]
        cleanup: Box<ScenarioRuntimeError>,
    },
}

impl ScenarioRuntimeError {
    /// Creates one action-port failure.
    #[must_use]
    pub fn action_port(operation: &'static str, detail: impl Into<String>) -> Self {
        Self::ActionPort {
            operation,
            detail: detail.into(),
        }
    }
}

/// Vehicle-specific action execution for one neutral scenario runtime.
pub trait ScenarioRuntime {
    /// Returns the final engine and vehicle action-port identity.
    fn identity(&self) -> &ArtifactIdentity;

    /// Returns all mission capabilities that the composed runtime supplies.
    fn capabilities(&self) -> &[MissionCapability];

    /// Prepares one admitted mission without starting external execution.
    fn prepare_blocking(
        &mut self,
        document: &pilotage_mission_core::MissionDocument,
        context: &RunExecutionContext,
    ) -> Result<(), ScenarioRuntimeError>;

    /// Starts the prepared vehicle action port.
    fn start_blocking(&mut self) -> Result<(), ScenarioRuntimeError>;

    /// Consumes one frame and advances an outstanding directive, if present.
    fn observe_blocking(
        &mut self,
        frame: &ScenarioFrame,
        directive: Option<&MissionDirective>,
    ) -> Result<ScenarioObservationReceipt, ScenarioRuntimeError>;

    /// Stops active action execution for the mission terminal result.
    fn stop_blocking(
        &mut self,
        context: &mut ScenarioStopContext,
    ) -> Result<(), ScenarioRuntimeError>;

    /// Restores the action port to a clean idle state.
    fn cleanup_blocking(&mut self) -> Result<(), ScenarioRuntimeError>;
}
