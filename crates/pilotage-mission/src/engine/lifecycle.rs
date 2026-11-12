//! The mission lifecycle machine: where the mission stands, from waiting
//! to start to what it does after its plan.
//!
//! One pure function, [`next`], turns a lifecycle and one event into the
//! next lifecycle. The events come from outside: the mission core says
//! which phase of its document it runs, and the host says when a human
//! takes the motion scope and hands it back. Whether the vehicle can stop
//! is carried by the event that ends the plan, so the machine reads no
//! configuration.
//!
//! The machine's state is the mission's state: the engine steps it after
//! every change of the core and on every hold and resume, and
//! [`MissionEngine::state`] reports it.

use pilotage_mission_core::{
    EngineState as CoreEngineState, MissionTerminal as CoreMissionTerminal, PhaseStage,
};

use super::document::{ARM_PHASE_ID, CLIMB_PHASE_ID};
use super::{MissionEngine, MissionState};

#[cfg(test)]
mod tests;

/// Where the mission stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(super) enum Stage {
    /// Waiting for what its first phase needs.
    #[default]
    Idle,
    /// The arm was requested.
    Armed,
    /// The climb is flown.
    Climb,
    /// The plan's route is flown.
    Route,
    /// The plan is done and the vehicle, which cannot stop, flies on.
    AfterPlan,
    /// The plan is done and the vehicle stopped.
    Complete,
    /// The mission failed or was refused.
    Failed,
}

/// The lifecycle: the stage, and whether a human holds the motion scope
/// over it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(super) struct Lifecycle {
    /// Where the mission stands.
    pub(super) stage: Stage,
    /// A human holds the motion scope: the stage waits.
    pub(super) suspended: bool,
}

/// What changed for the lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum LifecycleEvent {
    /// The core waits to start its first phase, or the climb.
    Waiting,
    /// The core runs the arm phase.
    Arming,
    /// The core runs the climb phase.
    Climbing,
    /// The core runs a phase of the route.
    OnRoute,
    /// The core finished the plan of a vehicle that can stop.
    PlanDoneStopped,
    /// The core finished the plan of a vehicle that cannot stop.
    PlanDoneFlying,
    /// The core failed, or refused the mission.
    Failed,
    /// A human took the motion scope.
    Held,
    /// The human handed the motion scope back.
    Resumed,
}

/// The lifecycle after `event`.
pub(super) const fn next(lifecycle: Lifecycle, event: LifecycleEvent) -> Lifecycle {
    use Stage::{AfterPlan, Armed, Climb, Complete, Failed, Idle, Route};
    let Lifecycle { stage, suspended } = lifecycle;
    match event {
        // A failure ends everything, a hold included.
        LifecycleEvent::Failed => Lifecycle {
            stage: Failed,
            suspended: false,
        },
        LifecycleEvent::Held => Lifecycle {
            stage,
            suspended: suspended || !matches!(stage, Complete | Failed),
        },
        LifecycleEvent::Resumed => Lifecycle {
            stage,
            suspended: false,
        },
        // The core does not run while a human holds the scope.
        _ if suspended => lifecycle,
        LifecycleEvent::Waiting => Lifecycle {
            stage: match stage {
                Idle | Armed => Idle,
                other => other,
            },
            suspended,
        },
        LifecycleEvent::Arming => Lifecycle {
            stage: match stage {
                Idle | Armed => Armed,
                other => other,
            },
            suspended,
        },
        LifecycleEvent::Climbing => Lifecycle {
            stage: match stage {
                Idle | Armed | Climb => Climb,
                other => other,
            },
            suspended,
        },
        LifecycleEvent::OnRoute => Lifecycle {
            stage: match stage {
                Idle | Armed | Climb | Route => Route,
                other => other,
            },
            suspended,
        },
        LifecycleEvent::PlanDoneStopped => Lifecycle {
            stage: match stage {
                Idle | Armed | Climb | Route => Complete,
                other => other,
            },
            suspended,
        },
        LifecycleEvent::PlanDoneFlying => Lifecycle {
            stage: match stage {
                Idle | Armed | Climb | Route => AfterPlan,
                other => other,
            },
            suspended,
        },
    }
}

impl Lifecycle {
    /// The mission state this lifecycle reports.
    pub(super) const fn reported(self) -> MissionState {
        if self.suspended {
            return MissionState::Held;
        }
        match self.stage {
            Stage::Idle => MissionState::AwaitSolution,
            Stage::Armed => MissionState::Arming,
            Stage::Climb => MissionState::Climb,
            Stage::Route => MissionState::Enroute,
            Stage::AfterPlan => MissionState::AfterPlan,
            Stage::Complete => MissionState::Complete,
            Stage::Failed => MissionState::Failed,
        }
    }
}

impl Lifecycle {
    /// Whether the vehicle is in flight under the mission: in its climb,
    /// on its route, or flying on after its plan.
    pub(super) const fn in_flight(self) -> bool {
        matches!(self.stage, Stage::Climb | Stage::Route | Stage::AfterPlan)
    }
}

impl MissionEngine {
    /// Steps the machine on what the mission core's state says now. The
    /// engine calls it after every change of the core, so
    /// [`MissionEngine::state`] is never behind it.
    pub(super) fn sync_lifecycle(&mut self) {
        if let Some(event) = self.core_event() {
            self.step_lifecycle(event);
        }
    }

    /// Steps the machine on `event`.
    pub(super) const fn step_lifecycle(&mut self, event: LifecycleEvent) {
        self.modes.lifecycle = next(self.modes.lifecycle, event);
    }

    /// Whether `event` would change the lifecycle.
    pub(super) fn lifecycle_takes(&self, event: LifecycleEvent) -> bool {
        next(self.modes.lifecycle, event) != self.modes.lifecycle
    }

    /// Where the mission stands.
    pub(super) const fn lifecycle(&self) -> Lifecycle {
        self.modes.lifecycle
    }

    /// What the mission core's state says of the lifecycle; `None` while
    /// a human holds the scope, when the core does not run.
    fn core_event(&self) -> Option<LifecycleEvent> {
        if self.core_failed {
            return Some(LifecycleEvent::Failed);
        }
        if self.hold_started.is_some() {
            return None;
        }
        let Some(core) = self.core.as_ref() else {
            return Some(LifecycleEvent::Waiting);
        };
        Some(match core.state() {
            CoreEngineState::Running {
                phase_id, stage, ..
            } if phase_id == ARM_PHASE_ID || phase_id == CLIMB_PHASE_ID => match stage {
                PhaseStage::WaitingForEntry {} => LifecycleEvent::Waiting,
                _ if phase_id == ARM_PHASE_ID => LifecycleEvent::Arming,
                _ => LifecycleEvent::Climbing,
            },
            CoreEngineState::Running { .. } => LifecycleEvent::OnRoute,
            CoreEngineState::Terminal {
                result: CoreMissionTerminal::Complete { .. },
            } if self.config.can_stop() => LifecycleEvent::PlanDoneStopped,
            CoreEngineState::Terminal {
                result: CoreMissionTerminal::Complete { .. },
            } => LifecycleEvent::PlanDoneFlying,
            CoreEngineState::CleaningUp { .. } | CoreEngineState::Terminal { .. } => {
                LifecycleEvent::Failed
            }
        })
    }
}
