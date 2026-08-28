use pilotage_mission_core::{
    DirectiveReceipt, EngineStart, EngineState, MissionDirective, MissionDocument, MissionEngine,
    TickInput, TickOutput,
};

use crate::{ArtifactIdentity, RunExecutionContext};

use super::{
    ScenarioFrame, ScenarioObservationReceipt, ScenarioRuntime, ScenarioRuntimeError,
    ScenarioStopContext, ScenarioStopReason,
};

/// The campaign host for one calibration mission engine instance.
pub struct CampaignMissionRuntime {
    document: MissionDocument,
    engine: Option<MissionEngine>,
    wall_duration_ns: u64,
    outstanding: Option<MissionDirective>,
    stopped: bool,
    cleaned: bool,
    last_source_sequence: Option<u64>,
    last_trial_time_ns: Option<u64>,
    last_wall_time_ns: Option<u64>,
}

impl CampaignMissionRuntime {
    /// Admits a mission and starts its attested action port.
    ///
    /// Identity and mission admission complete before the action port can
    /// prepare or start external execution.
    ///
    /// # Errors
    ///
    /// Returns an error when identity, admission, preparation, or startup fails.
    pub fn start_blocking<R: ScenarioRuntime>(
        document: MissionDocument,
        start: EngineStart,
        expected_identity: &ArtifactIdentity,
        runtime: &mut R,
        context: &RunExecutionContext,
    ) -> Result<Self, ScenarioRuntimeError> {
        let mut host = Self::admit(document, start)?;
        host.start_action_port_blocking(expected_identity, runtime, context)?;
        Ok(host)
    }

    /// Admits a mission without external action.
    ///
    /// # Errors
    ///
    /// Returns an error when the mission engine rejects the document or start input.
    pub fn admit(
        document: MissionDocument,
        start: EngineStart,
    ) -> Result<Self, ScenarioRuntimeError> {
        MissionEngine::start(document.clone(), start)
            .map_err(|source| ScenarioRuntimeError::EngineStart { source })?;
        Ok(Self {
            document,
            engine: None,
            wall_duration_ns: start
                .wall_deadline
                .expires_at_ns
                .saturating_sub(start.wall_time_ns),
            outstanding: None,
            stopped: false,
            cleaned: false,
            last_source_sequence: None,
            last_trial_time_ns: None,
            last_wall_time_ns: None,
        })
    }

    /// Attests and starts the action port for an admitted mission.
    ///
    /// # Errors
    ///
    /// Returns an error before preparation when the identity differs.
    pub fn start_action_port_blocking<R: ScenarioRuntime>(
        &mut self,
        expected_identity: &ArtifactIdentity,
        runtime: &mut R,
        context: &RunExecutionContext,
    ) -> Result<(), ScenarioRuntimeError> {
        self.prepare_action_port_blocking(expected_identity, runtime, context)?;
        self.start_prepared_action_port_blocking(runtime)
    }

    /// Attests and prepares the action port without starting it.
    ///
    /// # Errors
    ///
    /// Returns an error when admission, preparation, or containment fails.
    pub fn prepare_action_port_blocking<R: ScenarioRuntime>(
        &mut self,
        expected_identity: &ArtifactIdentity,
        runtime: &mut R,
        context: &RunExecutionContext,
    ) -> Result<(), ScenarioRuntimeError> {
        Self::attest_action_port(expected_identity, runtime)?;
        Self::attest_capabilities(&self.document, runtime)?;
        if let Err(primary) = runtime.prepare_blocking(&self.document, context) {
            let containment = self.cleanup_action_port_blocking(runtime);
            return combine_containment("prepare", primary, containment);
        }
        Ok(())
    }

    /// Starts one prepared action port.
    ///
    /// # Errors
    ///
    /// Returns an error when startup or containment fails.
    pub fn start_prepared_action_port_blocking<R: ScenarioRuntime>(
        &mut self,
        runtime: &mut R,
    ) -> Result<(), ScenarioRuntimeError> {
        if let Err(primary) = runtime.start_blocking() {
            let containment = self.stop_and_cleanup_blocking(
                runtime,
                ScenarioStopReason::ExecutionError,
                self.last_source_sequence,
            );
            return combine_containment("start", primary, containment);
        }
        Ok(())
    }

    /// Checks one action-port identity without external action.
    ///
    /// # Errors
    ///
    /// Returns an error when the expected or active identity is invalid or differs.
    pub fn attest_action_port<R: ScenarioRuntime>(
        expected_identity: &ArtifactIdentity,
        runtime: &R,
    ) -> Result<(), ScenarioRuntimeError> {
        expected_identity
            .validate()
            .map_err(|source| ScenarioRuntimeError::InvalidIdentity { source })?;
        runtime
            .identity()
            .validate()
            .map_err(|source| ScenarioRuntimeError::InvalidIdentity { source })?;
        if runtime.identity() != expected_identity {
            return Err(ScenarioRuntimeError::IdentityMismatch);
        }
        Ok(())
    }

    /// Checks all mission capabilities without external action.
    ///
    /// # Errors
    ///
    /// Returns an error when the runtime does not supply a required capability.
    pub fn attest_capabilities<R: ScenarioRuntime>(
        document: &MissionDocument,
        runtime: &R,
    ) -> Result<(), ScenarioRuntimeError> {
        for phase in &document.phases {
            for capability in &phase.required_capabilities {
                if !runtime.capabilities().contains(capability) {
                    return Err(ScenarioRuntimeError::MissingCapability {
                        phase_id: phase.id.clone(),
                        capability: *capability,
                    });
                }
            }
        }
        Ok(())
    }

    /// Advances the mission with one neutral frame.
    ///
    /// # Errors
    ///
    /// Returns an error when the frame, action receipt, or engine input is invalid.
    pub fn advance_blocking<R: ScenarioRuntime>(
        &mut self,
        runtime: &mut R,
        frame: &ScenarioFrame,
        wall_time_ns: u64,
    ) -> Result<TickOutput, ScenarioRuntimeError> {
        self.advance_authorized_blocking(runtime, frame, wall_time_ns, &mut || Ok(()))
    }

    pub(crate) fn advance_authorized_blocking<R, F>(
        &mut self,
        runtime: &mut R,
        frame: &ScenarioFrame,
        wall_time_ns: u64,
        authorize: &mut F,
    ) -> Result<TickOutput, ScenarioRuntimeError>
    where
        R: ScenarioRuntime,
        F: FnMut() -> Result<(), ScenarioRuntimeError>,
    {
        frame.mission_observation()?;
        self.validate_pre_action(frame, wall_time_ns)?;
        authorize()?;
        let observed = runtime.observe_blocking(frame, self.outstanding.as_ref())?;
        self.last_source_sequence = Some(frame.source_sequence);
        let receipts = self.correlate_receipt(frame, observed)?;
        let mut output = self.tick(frame, wall_time_ns, receipts)?;
        self.update_outstanding(&output)?;
        self.execute_emitted_blocking(runtime, frame, wall_time_ns, &mut output, authorize)?;
        self.last_trial_time_ns = Some(frame.trial_time_ns);
        self.last_wall_time_ns = Some(wall_time_ns);
        Ok(output)
    }

    /// Cleans the action port after terminal orchestration.
    ///
    /// # Errors
    ///
    /// Returns an error when cleanup fails.
    pub fn cleanup_blocking<R: ScenarioRuntime>(
        mut self,
        runtime: &mut R,
    ) -> Result<(), ScenarioRuntimeError> {
        self.cleanup_action_port_blocking(runtime)
    }

    /// Stops and cleans the action port for one campaign terminal reason.
    ///
    /// The method attempts cleanup when stop fails.
    ///
    /// # Errors
    ///
    /// Returns all stop and cleanup failures.
    pub fn stop_and_cleanup_blocking<R: ScenarioRuntime>(
        &mut self,
        runtime: &mut R,
        reason: ScenarioStopReason,
        last_source_sequence: Option<u64>,
    ) -> Result<(), ScenarioRuntimeError> {
        let stop = self.stop_action_port_blocking(runtime, reason, last_source_sequence);
        let cleanup = self.cleanup_action_port_blocking(runtime);
        match (stop, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(stop), Ok(())) => Err(stop),
            (Ok(()), Err(cleanup)) => Err(cleanup),
            (Err(stop), Err(cleanup)) => Err(ScenarioRuntimeError::StopAndCleanup {
                stop: Box::new(stop),
                cleanup: Box::new(cleanup),
            }),
        }
    }

    /// Returns the current mission engine state.
    #[must_use]
    pub fn state(&self) -> Option<EngineState> {
        self.engine.as_ref().map(MissionEngine::state)
    }

    /// Returns the last source sequence that the action port consumed.
    #[must_use]
    pub const fn last_consumed_source_sequence(&self) -> Option<u64> {
        self.last_source_sequence
    }

    fn execute_emitted_blocking<R, F>(
        &mut self,
        runtime: &mut R,
        frame: &ScenarioFrame,
        wall_time_ns: u64,
        output: &mut TickOutput,
        authorize: &mut F,
    ) -> Result<(), ScenarioRuntimeError>
    where
        R: ScenarioRuntime,
        F: FnMut() -> Result<(), ScenarioRuntimeError>,
    {
        let Some(directive) = output.directives.last().cloned() else {
            return Ok(());
        };
        authorize()?;
        let observed = runtime.observe_blocking(frame, Some(&directive))?;
        let receipts = self.correlate_receipt(frame, observed)?;
        if receipts.is_empty() {
            return Ok(());
        }
        let followup = self.tick(frame, wall_time_ns, receipts)?;
        self.update_outstanding(&followup)?;
        output.directives.extend(followup.directives);
        output.events.extend(followup.events);
        output.state = followup.state;
        Ok(())
    }

    fn tick(
        &mut self,
        frame: &ScenarioFrame,
        wall_time_ns: u64,
        receipts: Vec<DirectiveReceipt>,
    ) -> Result<TickOutput, ScenarioRuntimeError> {
        self.ensure_engine(frame, wall_time_ns)?
            .tick(TickInput {
                simulator_time_ns: frame.trial_time_ns,
                wall_time_ns,
                observation: frame.mission_observation()?,
                receipts,
            })
            .map_err(|source| ScenarioRuntimeError::EngineInput { source })
    }

    fn ensure_engine(
        &mut self,
        frame: &ScenarioFrame,
        wall_time_ns: u64,
    ) -> Result<&mut MissionEngine, ScenarioRuntimeError> {
        if self.engine.is_none() {
            let engine = MissionEngine::start(
                self.document.clone(),
                EngineStart {
                    host_target: pilotage_mission_core::ExecutionTarget::Simulator,
                    simulator_time_ns: frame.trial_time_ns,
                    wall_time_ns,
                    wall_deadline: pilotage_mission_core::WallDeadline {
                        mission_content_digest: self.document.identity.content_digest,
                        expires_at_ns: wall_time_ns.saturating_add(self.wall_duration_ns),
                    },
                },
            )
            .map_err(|source| ScenarioRuntimeError::EngineStart { source })?;
            self.engine = Some(engine);
        }
        self.engine
            .as_mut()
            .ok_or(ScenarioRuntimeError::EngineAbsent)
    }

    fn correlate_receipt(
        &self,
        frame: &ScenarioFrame,
        observed: ScenarioObservationReceipt,
    ) -> Result<Vec<DirectiveReceipt>, ScenarioRuntimeError> {
        if observed.source_sequence != frame.source_sequence {
            return Err(ScenarioRuntimeError::SourceSequenceMismatch {
                expected: frame.source_sequence,
                actual: observed.source_sequence,
            });
        }
        match (self.outstanding.as_ref(), observed.action_result) {
            (Some(directive), Some(result)) => Ok(vec![DirectiveReceipt {
                action_id: directive.context().action_id,
                result,
            }]),
            (None, Some(_)) => Err(ScenarioRuntimeError::UncorrelatedReceipt),
            (_, None) => Ok(Vec::new()),
        }
    }

    fn update_outstanding(&mut self, output: &TickOutput) -> Result<(), ScenarioRuntimeError> {
        if output.directives.len() > 1 {
            return Err(ScenarioRuntimeError::DirectiveCount {
                count: output.directives.len(),
            });
        }
        if let Some(directive) = output.directives.first() {
            self.outstanding = Some(directive.clone());
        } else if !waiting_for_receipt(&output.state) {
            self.outstanding = None;
        }
        Ok(())
    }

    fn stop_action_port_blocking<R: ScenarioRuntime>(
        &mut self,
        runtime: &mut R,
        reason: ScenarioStopReason,
        last_source_sequence: Option<u64>,
    ) -> Result<(), ScenarioRuntimeError> {
        if self.stopped {
            return Ok(());
        }
        let mut context = ScenarioStopContext {
            reason,
            last_source_sequence,
        };
        runtime.stop_blocking(&mut context)?;
        self.stopped = true;
        Ok(())
    }

    fn cleanup_action_port_blocking<R: ScenarioRuntime>(
        &mut self,
        runtime: &mut R,
    ) -> Result<(), ScenarioRuntimeError> {
        if self.cleaned {
            return Ok(());
        }
        runtime.cleanup_blocking()?;
        self.cleaned = true;
        Ok(())
    }

    fn validate_pre_action(
        &self,
        frame: &ScenarioFrame,
        wall_time_ns: u64,
    ) -> Result<(), ScenarioRuntimeError> {
        if self
            .last_source_sequence
            .is_some_and(|previous| frame.source_sequence <= previous)
        {
            return Err(invalid_frame("the source sequence did not increase"));
        }
        if self
            .last_trial_time_ns
            .is_some_and(|previous| frame.trial_time_ns < previous)
        {
            return Err(invalid_frame("the trial clock regressed"));
        }
        if self
            .last_wall_time_ns
            .is_some_and(|previous| wall_time_ns < previous)
        {
            return Err(invalid_frame("the wall clock regressed"));
        }
        Ok(())
    }
}

fn invalid_frame(detail: &str) -> ScenarioRuntimeError {
    ScenarioRuntimeError::InvalidFrame {
        detail: detail.to_owned(),
    }
}

fn combine_containment(
    operation: &'static str,
    primary: ScenarioRuntimeError,
    containment: Result<(), ScenarioRuntimeError>,
) -> Result<(), ScenarioRuntimeError> {
    match containment {
        Ok(()) => Err(primary),
        Err(containment) => Err(ScenarioRuntimeError::ActionAndContainment {
            operation,
            primary: Box::new(primary),
            containment: Box::new(containment),
        }),
    }
}

fn waiting_for_receipt(state: &EngineState) -> bool {
    matches!(
        state,
        EngineState::Running {
            stage: pilotage_mission_core::PhaseStage::WaitingForReceipt { .. },
            ..
        } | EngineState::CleaningUp {
            outstanding_action_id: Some(_),
            ..
        }
    )
}
