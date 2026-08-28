use flight_tune::{
    ArtifactIdentity, MissionArtifactIdentity, MissionCapability, MissionDirective,
    MissionDocument, ReceiptResult, RunExecutionContext, ScenarioFrame, ScenarioObservationReceipt,
    ScenarioRuntime, ScenarioRuntimeError, ScenarioStopContext, TrialAction,
};

/// One simulator-owned action from the shared mission directive contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XPlaneSimulatorAction {
    /// Reset the vehicle and the simulated world.
    Reset,
    /// Apply one immutable condition set.
    ApplyConditions {
        /// The exact condition-set identity.
        condition_set: MissionArtifactIdentity,
    },
}

/// X-Plane execution for simulator-owned mission actions.
pub trait XPlaneSimulatorActionDriver {
    /// Returns the mission capabilities that the simulator driver supplies.
    fn capabilities(&self) -> &[MissionCapability];

    /// Executes one simulator action for the current neutral frame.
    ///
    /// # Errors
    ///
    /// Returns an error when X-Plane cannot apply or verify the action.
    fn execute_blocking(
        &mut self,
        frame: &ScenarioFrame,
        action: &XPlaneSimulatorAction,
    ) -> Result<ReceiptResult, ScenarioRuntimeError>;
}

/// Dispatches simulator actions to X-Plane and vehicle actions to the vehicle port.
pub struct XPlaneScenarioRuntime<S, V> {
    simulator: S,
    vehicle: V,
    capabilities: Vec<MissionCapability>,
}

impl<S: XPlaneSimulatorActionDriver, V: ScenarioRuntime> XPlaneScenarioRuntime<S, V> {
    /// Creates one typed simulator and vehicle action dispatcher.
    #[must_use]
    pub fn new(simulator: S, vehicle: V) -> Self {
        let mut capabilities = vehicle.capabilities().to_vec();
        for capability in simulator.capabilities() {
            if !capabilities.contains(capability) {
                capabilities.push(*capability);
            }
        }
        Self {
            simulator,
            vehicle,
            capabilities,
        }
    }
}

impl<S, V> XPlaneScenarioRuntime<S, V> {
    /// Returns the simulator and vehicle ports.
    #[must_use]
    pub fn into_inner(self) -> (S, V) {
        (self.simulator, self.vehicle)
    }
}

impl<S, V> ScenarioRuntime for XPlaneScenarioRuntime<S, V>
where
    S: XPlaneSimulatorActionDriver,
    V: ScenarioRuntime,
{
    fn identity(&self) -> &ArtifactIdentity {
        self.vehicle.identity()
    }

    fn capabilities(&self) -> &[MissionCapability] {
        &self.capabilities
    }

    fn prepare_blocking(
        &mut self,
        document: &MissionDocument,
        context: &RunExecutionContext,
    ) -> Result<(), ScenarioRuntimeError> {
        self.vehicle.prepare_blocking(document, context)
    }

    fn start_blocking(&mut self) -> Result<(), ScenarioRuntimeError> {
        self.vehicle.start_blocking()
    }

    fn observe_blocking(
        &mut self,
        frame: &ScenarioFrame,
        directive: Option<&MissionDirective>,
    ) -> Result<ScenarioObservationReceipt, ScenarioRuntimeError> {
        let Some(action) = directive.and_then(simulator_action) else {
            return self.vehicle.observe_blocking(frame, directive);
        };
        let observed = self.vehicle.observe_blocking(frame, None)?;
        if observed.source_sequence != frame.source_sequence {
            return Err(ScenarioRuntimeError::SourceSequenceMismatch {
                expected: frame.source_sequence,
                actual: observed.source_sequence,
            });
        }
        if observed.action_result.is_some() {
            return Err(ScenarioRuntimeError::UncorrelatedReceipt);
        }
        Ok(ScenarioObservationReceipt {
            source_sequence: frame.source_sequence,
            action_result: Some(self.simulator.execute_blocking(frame, &action)?),
        })
    }

    fn stop_blocking(
        &mut self,
        context: &mut ScenarioStopContext,
    ) -> Result<(), ScenarioRuntimeError> {
        self.vehicle.stop_blocking(context)
    }

    fn cleanup_blocking(&mut self) -> Result<(), ScenarioRuntimeError> {
        self.vehicle.cleanup_blocking()
    }
}

fn simulator_action(directive: &MissionDirective) -> Option<XPlaneSimulatorAction> {
    let MissionDirective::Trial(directive) = directive else {
        return None;
    };
    match &directive.action {
        TrialAction::Reset {} => Some(XPlaneSimulatorAction::Reset),
        TrialAction::ApplyConditions { condition_set } => {
            Some(XPlaneSimulatorAction::ApplyConditions {
                condition_set: condition_set.clone(),
            })
        }
        TrialAction::WaitReady {}
        | TrialAction::ReachStartState { .. }
        | TrialAction::Settle {}
        | TrialAction::Stimulate { .. }
        | TrialAction::ReleaseControl {}
        | TrialAction::Observe {}
        | TrialAction::Stop {}
        | TrialAction::Disarm {}
        | TrialAction::CollectResults {} => None,
    }
}
