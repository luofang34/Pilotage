use flight_tune::{
    ArtifactIdentity, ControlChannel, ControlFamily, Digest, DirectiveContext, FlightAction,
    MissionCapability, MissionDirective, MissionDocument, ReceiptResult, RunExecutionContext,
    ScenarioFrame, ScenarioObservationReceipt, ScenarioRuntime, ScenarioRuntimeError,
    ScenarioStopContext, StartState, StimulusEnvelope, StimulusMapping, TrialAction, TuneError,
    Waveform, scenario_runtime_identity,
};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

const ACTION_PORT_IDENTITY_DOMAIN: &[u8] = b"pilotage-aviate-action-port-v2\0";

/// One Aviate vehicle action with mission correlation data.
#[derive(Debug, Clone, PartialEq)]
pub struct AviateVehicleDirective {
    /// The mission correlation and phase data.
    pub context: DirectiveContext,
    /// The vehicle action that Aviate must execute.
    pub action: AviateVehicleAction,
}

/// One typed action in the Aviate vehicle port.
#[derive(Debug, Clone, PartialEq)]
pub enum AviateVehicleAction {
    /// Arm the vehicle.
    Arm,
    /// Wait for valid estimator and command-link states.
    WaitReady,
    /// Move to the declared start state.
    ReachStartState {
        /// The declared reset-relative start state.
        target: StartState,
    },
    /// Hold the start state until the vehicle settles.
    Settle,
    /// Apply one typed control stimulus.
    Stimulate {
        /// The physical control family that the stimulus commands.
        family: ControlFamily,
        /// The control channel.
        channel: ControlChannel,
        /// The rule that resolves a normalized value to a physical command.
        mapping: StimulusMapping,
        /// The versioned physical envelope of the normalized range.
        envelope: StimulusEnvelope,
        /// The stimulus waveform.
        waveform: Waveform,
    },
    /// Release the active control stimulus.
    ReleaseControl,
    /// Keep the released or neutral hold state.
    Observe,
    /// Stop active trial control.
    Stop,
    /// Disarm the vehicle and revoke test authority.
    Disarm,
    /// Seal the vehicle result collection step.
    CollectResults,
}

/// An Aviate vehicle action-port operation failed.
#[derive(Debug, Error)]
pub enum AviateActionPortError {
    /// The action-port identity is not valid.
    #[error("the Aviate action-port identity is not valid: {source}")]
    InvalidIdentity {
        /// The identity validation failure.
        #[source]
        source: TuneError,
    },
    /// An Aviate driver operation failed.
    #[error("the Aviate action port failed during {operation}: {detail}")]
    Driver {
        /// The failed operation.
        operation: &'static str,
        /// The stable driver detail.
        detail: String,
    },
}

impl AviateActionPortError {
    /// Creates one driver failure.
    #[must_use]
    pub fn driver(operation: &'static str, detail: impl Into<String>) -> Self {
        Self::Driver {
            operation,
            detail: detail.into(),
        }
    }
}

/// Aviate-specific execution of typed mission directives.
///
/// The driver owns command mapping, hold state, direct control, ledger state,
/// and causal source selection. It supplies link and estimator values through
/// the neutral frame projection.
pub trait AviateActionDriver {
    /// Returns the exact production and configuration identity of this port.
    fn action_port_identity(&self) -> &ArtifactIdentity;

    /// Returns the mission capabilities that the Aviate driver supplies.
    fn capabilities(&self) -> &[MissionCapability];

    /// Prepares one admitted mission without external action.
    fn prepare_blocking(
        &mut self,
        document: &MissionDocument,
        context: &RunExecutionContext,
    ) -> Result<(), AviateActionPortError>;

    /// Starts causal source selection and vehicle action execution.
    fn start_blocking(&mut self) -> Result<(), AviateActionPortError>;

    /// Advances the vehicle action for one neutral frame.
    fn observe_blocking(
        &mut self,
        frame: &ScenarioFrame,
        directive: Option<&AviateVehicleDirective>,
    ) -> Result<Option<ReceiptResult>, AviateActionPortError>;

    /// Stops active vehicle action and seals its evidence.
    fn stop_blocking(
        &mut self,
        context: &mut ScenarioStopContext,
    ) -> Result<(), AviateActionPortError>;

    /// Restores the driver to an idle state.
    fn cleanup_blocking(&mut self) -> Result<(), AviateActionPortError>;
}

/// The Aviate projection for the neutral scenario-runtime action port.
pub struct AviateVehicleActionPort<D> {
    driver: D,
    runtime_identity: ArtifactIdentity,
}

impl<D: AviateActionDriver> AviateVehicleActionPort<D> {
    /// Creates an action port with the composed final runtime identity.
    ///
    /// # Errors
    ///
    /// Returns an error when the driver identity is not valid.
    pub fn new(driver: D) -> Result<Self, AviateActionPortError> {
        let action_port_identity = aviate_action_port_identity(driver.action_port_identity())
            .map_err(|source| AviateActionPortError::InvalidIdentity { source })?;
        let runtime_identity = scenario_runtime_identity(&action_port_identity)
            .map_err(|source| AviateActionPortError::InvalidIdentity { source })?;
        Ok(Self {
            driver,
            runtime_identity,
        })
    }

    /// Returns the Aviate driver.
    #[must_use]
    pub fn into_inner(self) -> D {
        self.driver
    }

    /// Returns the Aviate driver without consuming the port.
    #[must_use]
    pub const fn driver(&self) -> &D {
        &self.driver
    }
}

/// Composes the Aviate projection source with its driver identity.
///
/// A simulator vehicle factory must return this value from
/// `scenario_action_port_identity`.
///
/// # Errors
///
/// Returns an error when the driver identity is not valid.
pub fn aviate_action_port_identity(
    driver_identity: &ArtifactIdentity,
) -> Result<ArtifactIdentity, TuneError> {
    scenario_runtime_identity(driver_identity)?;
    let source = include_bytes!("action_port.rs");
    let mut hasher = Sha256::new();
    hasher.update(ACTION_PORT_IDENTITY_DOMAIN);
    hasher.update((source.len() as u64).to_le_bytes());
    hasher.update(source);
    hasher.update((driver_identity.id.len() as u64).to_le_bytes());
    hasher.update(driver_identity.id.as_bytes());
    hasher.update(driver_identity.digest.as_bytes());
    ArtifactIdentity::new(
        "pilotage-aviate-action-port-v2",
        Digest::from_bytes(hasher.finalize().into()),
    )
}

impl<D: AviateActionDriver> ScenarioRuntime for AviateVehicleActionPort<D> {
    fn identity(&self) -> &ArtifactIdentity {
        &self.runtime_identity
    }

    fn capabilities(&self) -> &[MissionCapability] {
        self.driver.capabilities()
    }

    fn prepare_blocking(
        &mut self,
        document: &MissionDocument,
        context: &RunExecutionContext,
    ) -> Result<(), ScenarioRuntimeError> {
        self.driver
            .prepare_blocking(document, context)
            .map_err(|error| action_error("prepare", error))
    }

    fn start_blocking(&mut self) -> Result<(), ScenarioRuntimeError> {
        self.driver
            .start_blocking()
            .map_err(|error| action_error("start", error))
    }

    fn observe_blocking(
        &mut self,
        frame: &ScenarioFrame,
        directive: Option<&MissionDirective>,
    ) -> Result<ScenarioObservationReceipt, ScenarioRuntimeError> {
        let projected = match directive.map(project_vehicle_directive).transpose() {
            Ok(projected) => projected,
            Err(result) => {
                return Ok(ScenarioObservationReceipt {
                    source_sequence: frame.source_sequence,
                    action_result: Some(result),
                });
            }
        };
        let action_result = self
            .driver
            .observe_blocking(frame, projected.as_ref())
            .map_err(|error| action_error("observe", error))?;
        Ok(ScenarioObservationReceipt {
            source_sequence: frame.source_sequence,
            action_result,
        })
    }

    fn stop_blocking(
        &mut self,
        context: &mut ScenarioStopContext,
    ) -> Result<(), ScenarioRuntimeError> {
        self.driver
            .stop_blocking(context)
            .map_err(|error| action_error("stop", error))
    }

    fn cleanup_blocking(&mut self) -> Result<(), ScenarioRuntimeError> {
        self.driver
            .cleanup_blocking()
            .map_err(|error| action_error("cleanup", error))
    }
}

fn project_vehicle_directive(
    directive: &MissionDirective,
) -> Result<AviateVehicleDirective, ReceiptResult> {
    let (context, action) = match directive {
        MissionDirective::Flight(directive) => {
            let action = match &directive.action {
                FlightAction::Arm {} => AviateVehicleAction::Arm,
                FlightAction::Disarm {} => AviateVehicleAction::Disarm,
                FlightAction::Climb { .. }
                | FlightAction::FollowPlan { .. }
                | FlightAction::MaintainTarget {}
                | FlightAction::Land {} => return Err(refused("unsupported operational action")),
            };
            (directive.context.clone(), action)
        }
        MissionDirective::Trial(directive) => {
            let action = match &directive.action {
                TrialAction::Reset {} | TrialAction::ApplyConditions { .. } => {
                    return Err(refused("simulator action reached the Aviate vehicle port"));
                }
                TrialAction::WaitReady {} => AviateVehicleAction::WaitReady,
                TrialAction::ReachStartState { target } => {
                    AviateVehicleAction::ReachStartState { target: *target }
                }
                TrialAction::Settle {} => AviateVehicleAction::Settle,
                TrialAction::Stimulate {
                    family,
                    channel,
                    mapping,
                    envelope,
                    waveform,
                } => AviateVehicleAction::Stimulate {
                    family: *family,
                    channel: *channel,
                    mapping: *mapping,
                    envelope: envelope.clone(),
                    waveform: waveform.clone(),
                },
                TrialAction::ReleaseControl {} => AviateVehicleAction::ReleaseControl,
                TrialAction::Observe {} => AviateVehicleAction::Observe,
                TrialAction::Stop {} => AviateVehicleAction::Stop,
                TrialAction::Disarm {} => AviateVehicleAction::Disarm,
                TrialAction::CollectResults {} => AviateVehicleAction::CollectResults,
            };
            (directive.context.clone(), action)
        }
    };
    Ok(AviateVehicleDirective { context, action })
}

fn refused(detail: &str) -> ReceiptResult {
    ReceiptResult::Refused {
        detail: detail.to_owned(),
    }
}

fn action_error(operation: &'static str, error: AviateActionPortError) -> ScenarioRuntimeError {
    ScenarioRuntimeError::action_port(operation, error.to_string())
}

#[cfg(test)]
mod tests;
