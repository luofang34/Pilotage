//! Phase actions and completion conditions.

use serde::{Deserialize, Serialize};

use super::{BackendCapability, Waveform};
use crate::{
    ArtifactIdentity, MAX_CAPABILITIES, MAX_CONDITION_VALUES, MAX_PHASE_CONDITIONS, MAX_TEXT_BYTES,
    ValidationError,
    validation::{count, duration, finite, range, text, unique},
};

/// A control channel for a stimulus.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlChannel {
    /// The roll channel.
    Roll,
    /// The pitch channel.
    Pitch,
    /// The vertical channel.
    Vertical,
    /// The yaw channel.
    Yaw,
}

/// A scalar comparison operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Comparison {
    /// The signal is less than the limit.
    LessThan,
    /// The signal is less than or equal to the limit.
    LessOrEqual,
    /// The signal is greater than the limit.
    GreaterThan,
    /// The signal is greater than or equal to the limit.
    GreaterOrEqual,
    /// The absolute signal is less than or equal to the limit.
    AbsoluteLessOrEqual,
    /// The absolute signal is greater than or equal to the limit.
    AbsoluteGreaterOrEqual,
}

/// A source of one scalar sample signal.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalSource {
    /// An axis in the raw input report.
    RawInputAxis,
    /// A normalized control axis.
    NormalizedControl,
    /// A scalar in the typed intent.
    TypedIntent,
    /// A scalar in the adapter demand.
    AdapterDemand,
    /// A scalar in the transmitted setpoint.
    TransmittedSetpoint,
    /// An estimated position component.
    EstimatePosition,
    /// An estimated velocity component.
    EstimateVelocity,
    /// An estimated acceleration component.
    EstimateAcceleration,
    /// An estimated attitude component.
    EstimateAttitude,
    /// An estimated body rate component.
    EstimateBodyRate,
    /// A truth position component.
    TruthPosition,
    /// A truth velocity component.
    TruthVelocity,
    /// A truth acceleration component.
    TruthAcceleration,
    /// A truth attitude component.
    TruthAttitude,
    /// A truth body rate component.
    TruthBodyRate,
    /// An actuator value.
    Actuator,
    /// A named environmental condition value.
    ConditionValue,
}

impl SignalSource {
    const fn component_count(self) -> usize {
        match self {
            Self::RawInputAxis => crate::MAX_RAW_AXES,
            Self::NormalizedControl => 4,
            Self::TypedIntent | Self::AdapterDemand | Self::TransmittedSetpoint => 8,
            Self::EstimateAttitude | Self::TruthAttitude => 4,
            Self::Actuator => crate::MAX_ACTUATOR_VALUES,
            Self::ConditionValue => MAX_CONDITION_VALUES,
            _ => 3,
        }
    }

    const fn required_capability(self) -> Option<BackendCapability> {
        match self {
            Self::TruthPosition
            | Self::TruthVelocity
            | Self::TruthAcceleration
            | Self::TruthAttitude
            | Self::TruthBodyRate => Some(BackendCapability::KinematicTruth),
            Self::ConditionValue => Some(BackendCapability::ConditionControl),
            _ => None,
        }
    }
}

/// A selector for one scalar in a trial sample.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignalSelector {
    /// The signal source.
    pub source: SignalSource,
    /// The zero-based component index.
    pub component: u16,
}

/// A condition for phase entry, exit, or abort.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PhaseCondition {
    /// The condition is always true.
    Always,
    /// The lifecycle state has the specified value.
    Lifecycle {
        /// The required lifecycle state.
        state: crate::LifecycleState,
    },
    /// The ground contact state has the specified value.
    GroundContact {
        /// The required ground contact state.
        expected: bool,
    },
    /// The crash state has the specified value.
    Crashed {
        /// The required crash state.
        expected: bool,
    },
    /// The control link validity has the specified value.
    LinkValid {
        /// The required link validity.
        expected: bool,
    },
    /// The estimator validity has the specified value.
    EstimatorValid {
        /// The required estimator validity.
        expected: bool,
    },
    /// The simulator time satisfies a comparison.
    SimulatorTime {
        /// The comparison operation.
        comparison: Comparison,
        /// The comparison time in nanoseconds.
        value_ns: u64,
    },
    /// A scalar sample signal satisfies a comparison.
    Signal {
        /// The selected scalar signal.
        selector: SignalSelector,
        /// The comparison operation.
        comparison: Comparison,
        /// The comparison value.
        value: f64,
    },
}

/// The action for one scenario phase.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PhaseAction {
    /// Reset the vehicle and the simulated world.
    Reset,
    /// Wait for the backend and vehicle to become ready.
    WaitReady,
    /// Apply one immutable environmental condition set.
    ApplyConditions {
        /// The condition set identity.
        condition_set: ArtifactIdentity,
    },
    /// Arm the vehicle.
    Arm,
    /// Move the vehicle to the test start state.
    ReachStartState {
        /// The target state relative to the reset observation.
        target: StartState,
    },
    /// Hold the start state before the stimulus.
    Settle,
    /// Apply one control stimulus.
    Stimulus {
        /// The control channel.
        channel: ControlChannel,
        /// The stimulus waveform.
        waveform: Waveform,
    },
    /// Release the test control input.
    ReleaseControl,
    /// Observe the response without a new stimulus.
    Observe,
    /// Stop active control.
    Stop,
    /// Disarm the vehicle.
    Disarm,
    /// Mark the data collection phase.
    CollectResults,
}

/// One bounded phase in a scenario.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Phase {
    /// The phase identifier.
    pub id: String,
    /// The maximum phase duration in simulator nanoseconds.
    pub max_sim_time_ns: u64,
    /// The backend capabilities that the phase needs.
    pub required_capabilities: Vec<BackendCapability>,
    /// The conditions that permit phase entry.
    pub entry_conditions: Vec<PhaseCondition>,
    /// The phase action.
    pub action: PhaseAction,
    /// The conditions that complete the phase.
    pub exit_conditions: Vec<PhaseCondition>,
    /// The conditions that abort the trial.
    pub abort_conditions: Vec<PhaseCondition>,
}

impl Phase {
    pub(crate) fn validate(&self, index: usize) -> Result<(), ValidationError> {
        let field = format!("scenario.phases[{index}]");
        text(&format!("{field}.id"), &self.id, MAX_TEXT_BYTES)?;
        duration(&format!("{field}.max_sim_time_ns"), self.max_sim_time_ns)?;
        count(
            &format!("{field}.required_capabilities"),
            self.required_capabilities.len(),
            MAX_CAPABILITIES,
        )?;
        unique(
            &format!("{field}.required_capabilities"),
            &self.required_capabilities,
        )?;
        self.validate_conditions(&field)?;
        self.validate_action(&field)?;
        self.validate_capability_declarations(&field)
    }

    fn validate_conditions(&self, field: &str) -> Result<(), ValidationError> {
        validate_condition_list(field, "entry_conditions", &self.entry_conditions)?;
        validate_condition_list(field, "exit_conditions", &self.exit_conditions)?;
        validate_condition_list(field, "abort_conditions", &self.abort_conditions)
    }

    fn validate_action(&self, field: &str) -> Result<(), ValidationError> {
        match &self.action {
            PhaseAction::ApplyConditions { condition_set } => {
                condition_set.validate(&format!("{field}.action.condition_set"))
            }
            PhaseAction::ReachStartState { target } => target.validate(field),
            PhaseAction::Stimulus { waveform, .. } => {
                waveform.validate(&format!("{field}.action.waveform"))
            }
            _ => Ok(()),
        }
    }

    fn validate_capability_declarations(&self, field: &str) -> Result<(), ValidationError> {
        self.require_declared(field, BackendCapability::SimulatorTime)?;
        if let Some(capability) = action_capability(&self.action) {
            self.require_declared(field, capability)?;
        }
        for condition in self
            .entry_conditions
            .iter()
            .chain(&self.exit_conditions)
            .chain(&self.abort_conditions)
        {
            if let Some(capability) = condition_capability(condition) {
                self.require_declared(field, capability)?;
            }
        }
        Ok(())
    }

    fn require_declared(
        &self,
        field: &str,
        capability: BackendCapability,
    ) -> Result<(), ValidationError> {
        if self.required_capabilities.contains(&capability) {
            return Ok(());
        }
        Err(ValidationError::UnsupportedCapability {
            phase: self.id.clone(),
            capability: format!("{} (not declared in {field})", capability.as_str()),
        })
    }
}

/// A test start state relative to the first observation after reset.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartState {
    /// The north-east-down position offset in meters.
    pub relative_position_ned_m: [f64; 3],
    /// The target heading.
    pub heading: StartHeading,
}

/// The heading reference for a test start state.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StartHeading {
    /// Add an offset to the first heading after reset.
    ResetOffset {
        /// The clockwise heading offset in radians.
        radians: f64,
    },
    /// Use a true heading clockwise from north.
    True {
        /// The true heading in radians.
        radians: f64,
    },
}

impl StartState {
    fn validate(&self, field: &str) -> Result<(), ValidationError> {
        for (index, value) in self.relative_position_ned_m.iter().enumerate() {
            range(
                &format!("{field}.action.target.relative_position_ned_m[{index}]"),
                *value,
                -1_000.0,
                1_000.0,
            )?;
        }
        self.heading.validate(field)
    }
}

impl StartHeading {
    fn validate(self, field: &str) -> Result<(), ValidationError> {
        let radians = match self {
            Self::ResetOffset { radians } | Self::True { radians } => radians,
        };
        range(
            &format!("{field}.action.target.heading.radians"),
            radians,
            -core::f64::consts::PI,
            core::f64::consts::PI,
        )
    }
}

fn validate_condition_list(
    field: &str,
    name: &str,
    conditions: &[PhaseCondition],
) -> Result<(), ValidationError> {
    count(
        &format!("{field}.{name}"),
        conditions.len(),
        MAX_PHASE_CONDITIONS,
    )?;
    for (index, condition) in conditions.iter().enumerate() {
        if let PhaseCondition::Signal {
            selector, value, ..
        } = condition
        {
            validate_selector(field, name, index, selector)?;
            finite(&format!("{field}.{name}[{index}].value"), *value)?;
        }
    }
    Ok(())
}

fn validate_selector(
    field: &str,
    name: &str,
    index: usize,
    selector: &SignalSelector,
) -> Result<(), ValidationError> {
    let limit = selector.source.component_count();
    if usize::from(selector.component) < limit {
        return Ok(());
    }
    Err(ValidationError::OutOfRange {
        field: format!("{field}.{name}[{index}].selector.component"),
        actual: f64::from(selector.component),
        minimum: 0.0,
        maximum: (limit.saturating_sub(1)) as f64,
    })
}

const fn action_capability(action: &PhaseAction) -> Option<BackendCapability> {
    match action {
        PhaseAction::Reset => Some(BackendCapability::Reset),
        PhaseAction::WaitReady => Some(BackendCapability::LifecycleState),
        PhaseAction::ApplyConditions { .. } => Some(BackendCapability::ConditionControl),
        PhaseAction::Arm | PhaseAction::Disarm => Some(BackendCapability::ArmDisarm),
        PhaseAction::ReachStartState { .. } => Some(BackendCapability::KinematicTruth),
        _ => None,
    }
}

const fn condition_capability(condition: &PhaseCondition) -> Option<BackendCapability> {
    match condition {
        PhaseCondition::Lifecycle { .. } => Some(BackendCapability::LifecycleState),
        PhaseCondition::GroundContact { .. } | PhaseCondition::Crashed { .. } => {
            Some(BackendCapability::ContactState)
        }
        PhaseCondition::SimulatorTime { .. } => Some(BackendCapability::SimulatorTime),
        PhaseCondition::Signal { selector, .. } => selector.source.required_capability(),
        _ => None,
    }
}
