//! Deterministic mission sequencing with caller-supplied input and clocks.

mod cleanup;
mod condition;
mod directive;
mod error;
mod observation;
mod outcome;
mod run;
mod runtime;

pub use directive::{
    ActionId, BoundDirective, DirectiveContext, DirectivePurpose, DirectiveReceipt,
    FlightDirective, MissionDirective, ReceiptResult, TrialDirective,
};
pub use error::{EngineInputError, EngineStartError};
pub use observation::{
    EngineStart, MissionObservation, NavigationObservation, ObservedSignal, TickInput,
    VehicleObservation, WallDeadline,
};
pub use outcome::{
    AbortCause, CleanupFailure, CleanupFailureKind, DeadlineClass, EngineEvent, EngineState,
    MissionTerminal, PhaseStage, TickOutput,
};

use crate::MissionDocument;
use crate::executor::ExecutorRegistry;
use runtime::{PhaseProgress, RunState, TickCollector, TickContext};

/// One deterministic mission sequencing core.
///
/// The core does not perform input or output. A host supplies time,
/// observations, and receipts to [`MissionEngine::tick`].
#[derive(Clone, Debug)]
pub struct MissionEngine {
    document: MissionDocument,
    wall_deadline: WallDeadline,
    last_mission_time_ns: u64,
    last_simulator_time_ns: Option<u64>,
    last_wall_time_ns: u64,
    last_action_id: u32,
    entered_phases: Vec<usize>,
    run_state: RunState,
}

impl MissionEngine {
    /// Validates a mission for a host with no executor and starts one run:
    /// a document with a bound action is refused.
    ///
    /// # Errors
    ///
    /// Returns an error if admission, identity, or wall-deadline validation
    /// fails.
    pub fn start(document: MissionDocument, start: EngineStart) -> Result<Self, EngineStartError> {
        Self::start_with_executors(document, start, ExecutorRegistry::NONE)
    }

    /// Validates a mission for a host whose executors `executors` registers,
    /// and starts one run. Every bound action must name a registered
    /// operation in its version, with parameters of the declared types.
    ///
    /// # Errors
    ///
    /// Returns an error if admission, executor admission, identity, or
    /// wall-deadline validation fails.
    pub fn start_with_executors(
        document: MissionDocument,
        start: EngineStart,
        executors: ExecutorRegistry,
    ) -> Result<Self, EngineStartError> {
        for phase in &document.phases {
            if let crate::MissionAction::Bound(action) = &phase.action {
                executors
                    .admit(action)
                    .map_err(|source| EngineStartError::Executor {
                        phase: phase.id.clone(),
                        source,
                    })?;
            }
        }
        document
            .validate_for_target(start.host_target)
            .map_err(|source| EngineStartError::Admission { source })?;
        document
            .verify_content_digest()
            .map_err(|source| EngineStartError::DocumentIdentity { source })?;
        validate_wall_deadline(&document, start)?;
        let first = document.next_ready(&[]).unwrap_or(0);
        Ok(Self {
            document,
            wall_deadline: start.wall_deadline,
            last_mission_time_ns: start.mission_time_ns,
            last_simulator_time_ns: start.simulator_time_ns,
            last_wall_time_ns: start.wall_time_ns,
            last_action_id: 0,
            entered_phases: Vec::new(),
            run_state: RunState::Running(runtime::RunningState {
                phase_index: first,
                phase_started: runtime::Clocks {
                    mission_ns: start.mission_time_ns,
                    simulator_ns: start.simulator_time_ns,
                },
                progress: PhaseProgress::Entry,
            }),
        })
    }

    /// Advances the engine with one complete caller input.
    ///
    /// # Errors
    ///
    /// Returns an error without changing engine state if a clock regresses,
    /// an observation is invalid, or a receipt does not match the outstanding
    /// directive.
    pub fn tick(&mut self, input: TickInput) -> Result<TickOutput, EngineInputError> {
        self.validate_input(&input)?;
        self.last_mission_time_ns = input.mission_time_ns;
        self.last_simulator_time_ns = input.simulator_time_ns;
        self.last_wall_time_ns = input.wall_time_ns;
        let mut collector = TickCollector::default();
        let context = TickContext {
            document: &self.document,
            entered_phases: &mut self.entered_phases,
            last_action_id: &mut self.last_action_id,
            input: &input,
            collector: &mut collector,
        };
        let transition = match &mut self.run_state {
            RunState::Running(state) => run::tick_running(self.wall_deadline, state, context),
            RunState::CleaningUp(state) => cleanup::tick_cleanup(state, context),
            RunState::Terminal(_) => None,
        };
        if let Some(state) = transition {
            self.run_state = state;
        }
        Ok(TickOutput {
            directives: collector.directives,
            events: collector.events,
            state: self.state(),
        })
    }

    /// Gets a public snapshot of the current engine state.
    #[must_use]
    pub fn state(&self) -> EngineState {
        match &self.run_state {
            RunState::Running(state) => runtime::running_snapshot(&self.document, state),
            RunState::CleaningUp(state) => runtime::cleanup_snapshot(state),
            RunState::Terminal(result) => EngineState::Terminal {
                result: result.clone(),
            },
        }
    }

    fn validate_input(&self, input: &TickInput) -> Result<(), EngineInputError> {
        if matches!(self.run_state, RunState::Terminal(_)) {
            return Err(EngineInputError::Terminal {});
        }
        validate_clocks(self, input)?;
        input.observation.validate()?;
        validate_receipts(input, self.outstanding_action_id())
    }

    fn current_phase(&self) -> Option<&crate::MissionPhase> {
        match &self.run_state {
            RunState::Running(state) => self.document.phases.get(state.phase_index),
            RunState::CleaningUp(_) | RunState::Terminal(_) => None,
        }
    }

    fn outstanding_action_id(&self) -> Option<ActionId> {
        match &self.run_state {
            RunState::Running(state) => match &state.progress {
                PhaseProgress::Receipt(pending) => Some(pending.directive.context().action_id),
                PhaseProgress::Entry | PhaseProgress::Completion => None,
            },
            RunState::CleaningUp(state) => state
                .pending
                .as_ref()
                .map(|pending| pending.directive.context().action_id),
            RunState::Terminal(_) => None,
        }
    }
}

fn validate_wall_deadline(
    document: &MissionDocument,
    start: EngineStart,
) -> Result<(), EngineStartError> {
    let expected = document.identity.content_digest;
    let actual = start.wall_deadline.mission_content_digest;
    if actual != expected {
        return Err(EngineStartError::WallDeadlineIdentity { expected, actual });
    }
    if start.wall_deadline.expires_at_ns <= start.wall_time_ns {
        return Err(EngineStartError::WallDeadlineExpired {
            wall_time_ns: start.wall_time_ns,
            deadline_ns: start.wall_deadline.expires_at_ns,
        });
    }
    Ok(())
}

fn validate_clocks(engine: &MissionEngine, input: &TickInput) -> Result<(), EngineInputError> {
    if input.mission_time_ns < engine.last_mission_time_ns {
        return Err(EngineInputError::MissionClockRegressed {
            previous_ns: engine.last_mission_time_ns,
            current_ns: input.mission_time_ns,
        });
    }
    if let (Some(previous_ns), Some(current_ns)) =
        (engine.last_simulator_time_ns, input.simulator_time_ns)
        && current_ns < previous_ns
    {
        return Err(EngineInputError::SimulatorClockRegressed {
            previous_ns,
            current_ns,
        });
    }
    if input.simulator_time_ns.is_none()
        && let Some(phase) = engine.current_phase()
        && phase.needs_simulator_time()
    {
        return Err(EngineInputError::SimulatorTimeMissing {
            phase_id: phase.id.clone(),
        });
    }
    if input.wall_time_ns < engine.last_wall_time_ns {
        return Err(EngineInputError::WallClockRegressed {
            previous_ns: engine.last_wall_time_ns,
            current_ns: input.wall_time_ns,
        });
    }
    Ok(())
}

fn validate_receipts(
    input: &TickInput,
    outstanding: Option<ActionId>,
) -> Result<(), EngineInputError> {
    if input.receipts.len() > 1 {
        return Err(EngineInputError::TooManyReceipts {
            count: input.receipts.len(),
        });
    }
    if let Some(receipt) = input.receipts.first()
        && Some(receipt.action_id) != outstanding
    {
        return Err(EngineInputError::StaleReceipt {
            received: receipt.action_id,
            outstanding,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
