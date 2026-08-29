//! The vehicle-side phase machine of one Aviate run.
//!
//! One directive is open at a time. The machine latches its entry frame,
//! advances it on every accepted frame, and returns a receipt only when
//! the directive is actually finished. A directive that is not finished
//! returns nothing, which is what lets the mission engine keep waiting
//! instead of scoring a window that has not closed.

pub mod direct;
pub mod transition;
pub mod waveform;

use flight_tune::{
    ControlChannel, ReceiptResult, ScenarioFrame, StartState, VehicleLifecycleState, Waveform,
};

use crate::action_port::{AviateVehicleAction, AviateVehicleDirective};

use super::AviateRuntimeError;
use super::conditions::ConditionLedger;
use super::direct::{DirectControl, DirectEntryState};
use super::timing::{FrameStamp, PhaseClock};
use direct::{DirectStepOutcome, step_request};
use transition::{SettleWindow, StartOrigin, StartStateTolerance, start_state_error};
use waveform::WaveformSample;

/// How a directive advanced on one frame.
#[derive(Clone, Debug, PartialEq)]
pub enum PhaseProgress {
    /// The directive is still running. The engine keeps waiting.
    Running,
    /// The directive finished with this receipt.
    Complete(ReceiptResult),
}

impl PhaseProgress {
    /// One directive that finished successfully.
    #[must_use]
    pub const fn succeeded() -> Self {
        Self::Complete(ReceiptResult::Succeeded {})
    }

    /// One directive the vehicle port cannot admit.
    #[must_use]
    pub fn refused(detail: impl Into<String>) -> Self {
        Self::Complete(ReceiptResult::Refused {
            detail: detail.into(),
        })
    }

    /// One directive that is still running.
    #[must_use]
    pub const fn running() -> Self {
        Self::Running
    }
}

/// What one control stimulus asks for on one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StimulusAdvance {
    /// The waveform commands this normalized value on this frame.
    Command {
        /// The bounded normalized value to command.
        normalized: f64,
        /// Whether the directive finishes on this same frame.
        complete: bool,
    },
    /// The waveform reached the end of its own declared window.
    Complete,
}

/// The vehicle-side progress of one run's directives.
#[derive(Clone, Copy, Debug)]
pub struct PhaseMachine {
    clock: PhaseClock,
    origin: StartOrigin,
    settle: SettleWindow,
    tolerance: StartStateTolerance,
}

impl PhaseMachine {
    /// Creates one machine with the declared start-state tolerance.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when a tolerance bound is unusable.
    pub fn new(tolerance: StartStateTolerance) -> Result<Self, AviateRuntimeError> {
        tolerance.validate()?;
        Ok(Self {
            clock: PhaseClock::new(),
            origin: StartOrigin::new(),
            settle: SettleWindow::new(),
            tolerance,
        })
    }

    /// Clears every latched window for a new run.
    pub const fn reset(&mut self) {
        self.clock.leave();
        self.settle.reset();
        self.origin.clear();
    }

    /// Latches the reset-relative origin from the first frame of a run.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the frame attitude is invalid.
    pub fn latch_origin(&mut self, frame: &ScenarioFrame) -> Result<(), AviateRuntimeError> {
        self.origin.latch(frame)
    }

    /// Closes the open directive so the next one opens its own window.
    pub const fn close(&mut self) {
        self.clock.leave();
        self.settle.reset();
    }

    /// The trial nanoseconds since the open directive latched its entry.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when no directive is open.
    pub fn elapsed_ns(&self, stamp: FrameStamp) -> Result<u64, AviateRuntimeError> {
        self.clock.elapsed_ns(stamp)
    }

    /// Opens one directive on the frame that carries it.
    pub const fn open(&mut self, stamp: FrameStamp) {
        self.clock.enter(stamp);
    }

    /// Whether the vehicle reports both link and estimator as valid.
    #[must_use]
    pub fn is_ready(frame: &ScenarioFrame) -> bool {
        frame.link_valid == Some(true) && frame.estimator_valid == Some(true)
    }

    /// Advances a wait for valid link and estimator states.
    #[must_use]
    pub fn advance_wait_ready(&self, frame: &ScenarioFrame) -> PhaseProgress {
        if Self::is_ready(frame) {
            return PhaseProgress::succeeded();
        }
        PhaseProgress::running()
    }

    /// Advances an arm or disarm request toward its lifecycle state.
    #[must_use]
    pub fn advance_lifecycle(
        frame: &ScenarioFrame,
        expected: VehicleLifecycleState,
    ) -> PhaseProgress {
        if frame.lifecycle == Some(expected) {
            return PhaseProgress::succeeded();
        }
        PhaseProgress::running()
    }

    /// Advances the move toward one declared start state.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the origin is not latched or a
    /// frame value is invalid.
    pub fn advance_start_state(
        &mut self,
        stamp: FrameStamp,
        frame: &ScenarioFrame,
        target: &StartState,
    ) -> Result<PhaseProgress, AviateRuntimeError> {
        let error = start_state_error(&self.origin, frame, target)?;
        let within = error.is_within(&self.tolerance);
        // Reaching the start state is a position check, not a dwell: the
        // settle directive is what proves the vehicle can hold it.
        let _ = self.settle.advance(stamp, within, 0)?;
        Ok(if within {
            PhaseProgress::succeeded()
        } else {
            PhaseProgress::running()
        })
    }

    /// Advances the dwell that proves the vehicle holds its start state.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the origin is not latched or a
    /// frame value is invalid.
    pub fn advance_settle(
        &mut self,
        stamp: FrameStamp,
        frame: &ScenarioFrame,
        target: &StartState,
    ) -> Result<PhaseProgress, AviateRuntimeError> {
        let error = start_state_error(&self.origin, frame, target)?;
        let within = error.is_within(&self.tolerance);
        if self
            .settle
            .advance(stamp, within, self.tolerance.dwell_ns)?
        {
            return Ok(PhaseProgress::succeeded());
        }
        Ok(PhaseProgress::running())
    }

    /// Advances one control stimulus and reports what it commands now.
    ///
    /// A waveform that states its own window finishes when that window
    /// closes. A step states no window: it is one commanded value, so the
    /// directive finishes on the frame that commands it and the hold keeps
    /// the value until the run releases control.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when no directive is open, when a
    /// value is not finite, or when the waveform cannot be resolved.
    pub fn advance_stimulus(
        &self,
        stamp: FrameStamp,
        wave: &Waveform,
    ) -> Result<StimulusAdvance, AviateRuntimeError> {
        let elapsed_ns = self.clock.elapsed_ns(stamp)?;
        Ok(match waveform::sample(wave, elapsed_ns)? {
            WaveformSample::Complete => StimulusAdvance::Complete,
            WaveformSample::Active(normalized) => StimulusAdvance::Command {
                normalized,
                complete: matches!(wave, Waveform::Step { .. }),
            },
        })
    }
}

/// What one advanced directive commanded and observed.
#[derive(Clone, Debug, PartialEq)]
pub struct DirectiveAdvance {
    /// How the directive advanced.
    pub progress: PhaseProgress,
    /// The normalized value the vehicle commanded on this frame.
    pub commanded: Option<f64>,
    /// The control channel the commanded value moves.
    pub channel: Option<ControlChannel>,
    /// Whether the commanded value reached a declared envelope endpoint.
    pub saturated: bool,
}

impl DirectiveAdvance {
    fn of(progress: PhaseProgress) -> Self {
        Self {
            progress,
            commanded: None,
            channel: None,
            saturated: false,
        }
    }
}

/// Advances one Aviate vehicle directive on one accepted frame.
///
/// The dispatch is the only place that turns a mission action into a
/// vehicle command, so every action the port admits is answered here and
/// every action it does not admit is refused with a stated reason.
///
/// # Errors
///
/// Returns [`AviateRuntimeError`] when a frame value is invalid, when the
/// waveform cannot be resolved, or when the direct path refuses a command.
pub fn advance<D: DirectControl>(
    machine: &mut PhaseMachine,
    conditions: &mut ConditionLedger,
    direct: &mut D,
    stamp: FrameStamp,
    frame: &ScenarioFrame,
    directive: &AviateVehicleDirective,
) -> Result<DirectiveAdvance, AviateRuntimeError> {
    match &directive.action {
        AviateVehicleAction::Arm => Ok(DirectiveAdvance::of(if PhaseMachine::is_ready(frame) {
            PhaseMachine::advance_lifecycle(frame, VehicleLifecycleState::Armed)
        } else {
            PhaseProgress::running()
        })),
        AviateVehicleAction::WaitReady => {
            Ok(DirectiveAdvance::of(machine.advance_wait_ready(frame)))
        }
        AviateVehicleAction::ReachStartState { target } => Ok(DirectiveAdvance::of(
            machine.advance_start_state(stamp, frame, target)?,
        )),
        AviateVehicleAction::Settle => Ok(DirectiveAdvance::of(machine.advance_settle(
            stamp,
            frame,
            &settle_target(),
        )?)),
        AviateVehicleAction::Stimulate {
            family,
            channel,
            mapping,
            envelope,
            waveform,
        } => {
            conditions.lock();
            let request_for =
                |normalized| step_request(*family, *channel, *mapping, envelope, normalized);
            match machine.advance_stimulus(stamp, waveform)? {
                StimulusAdvance::Complete => Ok(DirectiveAdvance::of(PhaseProgress::succeeded())),
                StimulusAdvance::Command {
                    normalized,
                    complete,
                } => {
                    let request = request_for(normalized)?;
                    direct.ensure_baseline_blocking(entry_state(frame)?)?;
                    let outcome = direct.command_blocking(&request, false)?;
                    Ok(DirectiveAdvance {
                        progress: stimulus_progress(outcome, complete),
                        commanded: Some(normalized),
                        channel: Some(*channel),
                        saturated: normalized.abs() >= 1.0,
                    })
                }
            }
        }
        AviateVehicleAction::ReleaseControl => {
            conditions.unlock();
            Ok(DirectiveAdvance::of(PhaseProgress::succeeded()))
        }
        AviateVehicleAction::Observe => Ok(DirectiveAdvance::of(PhaseProgress::succeeded())),
        AviateVehicleAction::Stop => {
            conditions.unlock();
            direct.revoke();
            Ok(DirectiveAdvance::of(PhaseProgress::succeeded()))
        }
        AviateVehicleAction::Disarm => Ok(DirectiveAdvance::of(PhaseMachine::advance_lifecycle(
            frame,
            VehicleLifecycleState::Disarmed,
        ))),
        AviateVehicleAction::CollectResults => Ok(DirectiveAdvance::of(PhaseProgress::succeeded())),
    }
}

/// The progress one direct command outcome reports.
///
/// A command that sent nothing keeps the directive open: the run waits for
/// the raw source rather than scoring a step the flight controller never
/// received.
fn stimulus_progress(outcome: DirectStepOutcome, complete: bool) -> PhaseProgress {
    match outcome {
        DirectStepOutcome::Enacted if complete => PhaseProgress::succeeded(),
        DirectStepOutcome::Enacted | DirectStepOutcome::Pending => PhaseProgress::running(),
        DirectStepOutcome::NoExactSource => {
            PhaseProgress::refused("the raw source carries no exact sample for the command time")
        }
    }
}

/// The vehicle state that one direct stimulus is entered from.
///
/// # Errors
///
/// Returns [`AviateRuntimeError`] when the frame attitude is invalid.
fn entry_state(frame: &ScenarioFrame) -> Result<DirectEntryState, AviateRuntimeError> {
    let attitude = frame.truth.attitude_wxyz;
    Ok(DirectEntryState {
        roll_rad: crate::runtime::math::roll_rad(attitude)?,
        pitch_rad: crate::runtime::math::pitch_rad(attitude)?,
        yaw_rad: crate::runtime::math::yaw_rad(attitude)?,
    })
}

/// The state a settle directive holds.
///
/// A settle keeps the state the run already reached, so it measures the
/// origin the reach latched rather than a second declared target.
const fn settle_target() -> StartState {
    StartState {
        relative_position_ned_m: [0.0; 3],
        heading: flight_tune::StartHeading::ResetOffset { radians: 0.0 },
    }
}
