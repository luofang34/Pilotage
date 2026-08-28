//! Mission action vocabularies and transport lanes.

use serde::{Deserialize, Serialize};

use crate::{ArtifactIdentity, ControlChannel};
use crate::{
    FlightPlanReference, MissionCapability, ValidationError,
    trial::{StartState, Waveform},
};

/// A transport lane for one mission action.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum TransportLane {
    /// The action uses the operational command path.
    Operational,
    /// The action uses simulator-only authority.
    SimulatorOnly,
}

/// One action in a mission phase.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "domain",
    content = "action",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum MissionAction {
    /// An operational flight action.
    Flight(FlightAction),
    /// A simulator-only calibration action.
    Trial(TrialAction),
}

/// An operational flight action.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FlightAction {
    /// Arm the vehicle.
    Arm {},
    /// Climb to an altitude.
    Climb {
        /// The target altitude in meters.
        target_altitude_m: f64,
    },
    /// Follow an immutable flight plan.
    FollowPlan {
        /// The flight plan reference.
        plan: FlightPlanReference,
    },
    /// Hold the current flight target.
    Hold {},
    /// Land the vehicle.
    Land {},
    /// Disarm the vehicle.
    Disarm {},
}

/// A simulator-only calibration action.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TrialAction {
    /// Reset the vehicle and simulated world.
    Reset {},
    /// Wait for the simulator and vehicle to become ready.
    WaitReady {},
    /// Apply one immutable environmental condition set.
    ApplyConditions {
        /// The condition set identity.
        condition_set: ArtifactIdentity,
    },
    /// Move the vehicle to the trial start state.
    ReachStartState {
        /// The target state relative to the reset observation.
        target: StartState,
    },
    /// Hold the start state before the stimulus.
    Settle {},
    /// Apply one control stimulus.
    Stimulate {
        /// The control channel.
        channel: ControlChannel,
        /// The stimulus waveform.
        waveform: Waveform,
    },
    /// Release the trial control input.
    ReleaseControl {},
    /// Observe the response without a new stimulus.
    Observe {},
    /// Stop active trial control.
    Stop {},
    /// Disarm the vehicle.
    Disarm {},
    /// Mark the data collection phase.
    CollectResults {},
}

impl MissionAction {
    /// Gets the transport lane fixed by the action domain.
    #[must_use]
    pub const fn transport_lane(&self) -> TransportLane {
        match self {
            Self::Flight(_) => TransportLane::Operational,
            Self::Trial(_) => TransportLane::SimulatorOnly,
        }
    }

    pub(crate) fn validate(&self, field: &str) -> Result<(), ValidationError> {
        match self {
            Self::Flight(action) => action.validate(field),
            Self::Trial(action) => action.validate(field),
        }
    }

    pub(crate) const fn required_capability(&self) -> Option<MissionCapability> {
        match self {
            Self::Flight(action) => action.required_capability(),
            Self::Trial(action) => action.required_capability(),
        }
    }

    pub(crate) const fn name(&self) -> &'static str {
        match self {
            Self::Flight(action) => action.name(),
            Self::Trial(action) => action.name(),
        }
    }

    pub(crate) fn flight_plan(&self) -> Option<&FlightPlanReference> {
        match self {
            Self::Flight(FlightAction::FollowPlan { plan }) => Some(plan),
            _ => None,
        }
    }
}

impl FlightAction {
    fn validate(&self, field: &str) -> Result<(), ValidationError> {
        match self {
            Self::Climb { target_altitude_m } => crate::validation::finite(
                &format!("{field}.action.target_altitude_m"),
                *target_altitude_m,
            ),
            Self::FollowPlan { plan } => plan.validate(&format!("{field}.action.plan")),
            _ => Ok(()),
        }
    }

    const fn required_capability(&self) -> Option<MissionCapability> {
        match self {
            Self::Arm {} | Self::Disarm {} => Some(MissionCapability::ArmDisarm),
            Self::Climb { .. } | Self::Hold {} | Self::Land {} => {
                Some(MissionCapability::FlightControl)
            }
            Self::FollowPlan { .. } => Some(MissionCapability::FlightPlan),
        }
    }

    pub(crate) const fn name(&self) -> &'static str {
        match self {
            Self::Arm {} => "flight.arm",
            Self::Climb { .. } => "flight.climb",
            Self::FollowPlan { .. } => "flight.follow_plan",
            Self::Hold {} => "flight.hold",
            Self::Land {} => "flight.land",
            Self::Disarm {} => "flight.disarm",
        }
    }
}

impl TrialAction {
    fn validate(&self, field: &str) -> Result<(), ValidationError> {
        match self {
            Self::ApplyConditions { condition_set } => {
                condition_set.validate(&format!("{field}.action.condition_set"))
            }
            Self::ReachStartState { target } => target.validate(field),
            Self::Stimulate { waveform, .. } => {
                waveform.validate(&format!("{field}.action.waveform"))
            }
            _ => Ok(()),
        }
    }

    const fn required_capability(&self) -> Option<MissionCapability> {
        match self {
            Self::Reset {} => Some(MissionCapability::Reset),
            Self::WaitReady {} => Some(MissionCapability::LifecycleState),
            Self::ApplyConditions { .. } => Some(MissionCapability::ConditionControl),
            Self::ReachStartState { .. } => Some(MissionCapability::KinematicTruth),
            Self::Stimulate { .. } | Self::ReleaseControl {} | Self::Stop {} => {
                Some(MissionCapability::SimulatorControl)
            }
            Self::Disarm {} => Some(MissionCapability::ArmDisarm),
            Self::Settle {} | Self::Observe {} | Self::CollectResults {} => None,
        }
    }

    pub(crate) const fn name(&self) -> &'static str {
        match self {
            Self::Reset {} => "trial.reset",
            Self::WaitReady {} => "trial.wait_ready",
            Self::ApplyConditions { .. } => "trial.apply_conditions",
            Self::ReachStartState { .. } => "trial.reach_start_state",
            Self::Settle {} => "trial.settle",
            Self::Stimulate { .. } => "trial.stimulate",
            Self::ReleaseControl {} => "trial.release_control",
            Self::Observe {} => "trial.observe",
            Self::Stop {} => "trial.stop",
            Self::Disarm {} => "trial.disarm",
            Self::CollectResults {} => "trial.collect_results",
        }
    }
}
