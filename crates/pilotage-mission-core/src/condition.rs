//! Mission phase condition vocabularies.

use serde::{Deserialize, Serialize};

use crate::{MissionCapability, SignalSelector, ValidationError, validation};

/// A scalar comparison operation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Comparison {
    /// The value is less than the limit.
    LessThan,
    /// The value is less than or equal to the limit.
    LessOrEqual,
    /// The value is greater than the limit.
    GreaterThan,
    /// The value is greater than or equal to the limit.
    GreaterOrEqual,
    /// The absolute value is less than or equal to the limit.
    AbsoluteLessOrEqual,
    /// The absolute value is greater than or equal to the limit.
    AbsoluteGreaterOrEqual,
}

/// A condition for phase entry, completion, or abort.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "domain",
    content = "condition",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum MissionCondition {
    /// The condition is always true.
    Always {},
    /// A navigation condition.
    Navigation(NavigationCondition),
    /// A vehicle condition.
    Vehicle(VehicleCondition),
    /// A simulator condition.
    Simulator(SimulatorCondition),
    /// A scalar signal condition.
    Signal(SignalCondition),
}

/// A navigation condition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NavigationCondition {
    /// Navigation guidance has the specified validity.
    GuidanceValid {
        /// The required validity.
        expected: bool,
    },
    /// The active flight plan has the specified completion state.
    PlanComplete {
        /// The required completion state.
        expected: bool,
    },
    /// The observed altitude satisfies a comparison.
    Altitude {
        /// The comparison operation.
        comparison: Comparison,
        /// The altitude in meters.
        value_m: f64,
    },
}

/// A vehicle lifecycle state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum VehicleLifecycleState {
    /// The vehicle is stopped.
    Stopped,
    /// The vehicle is resetting.
    Resetting,
    /// The vehicle state is converging.
    Converging,
    /// The vehicle is ready.
    Ready,
    /// The vehicle is armed.
    Armed,
    /// The vehicle is disarmed.
    Disarmed,
    /// The vehicle is stopping.
    Stopping,
}

/// A vehicle condition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VehicleCondition {
    /// The lifecycle state has the specified value.
    Lifecycle {
        /// The required lifecycle state.
        state: VehicleLifecycleState,
    },
    /// The ground-contact state has the specified value.
    GroundContact {
        /// The required ground-contact state.
        expected: bool,
    },
    /// The crash state has the specified value.
    Crashed {
        /// The required crash state.
        expected: bool,
    },
    /// The control-link validity has the specified value.
    LinkValid {
        /// The required link validity.
        expected: bool,
    },
    /// The estimator validity has the specified value.
    EstimatorValid {
        /// The required estimator validity.
        expected: bool,
    },
}

/// A simulator condition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SimulatorCondition {
    /// Simulator time satisfies a comparison.
    Time {
        /// The comparison operation.
        comparison: Comparison,
        /// The comparison time in nanoseconds.
        value_ns: u64,
    },
}

/// A scalar signal condition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SignalCondition {
    /// A selected signal satisfies a comparison.
    Value {
        /// The selected scalar signal.
        selector: SignalSelector,
        /// The comparison operation.
        comparison: Comparison,
        /// The comparison value.
        value: f64,
    },
}

impl MissionCondition {
    pub(crate) fn validate(&self, field: &str) -> Result<(), ValidationError> {
        match self {
            Self::Always {} | Self::Vehicle(_) | Self::Simulator(_) => Ok(()),
            Self::Navigation(condition) => condition.validate(field),
            Self::Signal(condition) => condition.validate(field),
        }
    }

    pub(crate) const fn required_capability(&self) -> Option<MissionCapability> {
        match self {
            Self::Always {} => None,
            Self::Navigation(_) => Some(MissionCapability::NavigationState),
            Self::Vehicle(condition) => condition.required_capability(),
            Self::Simulator(_) => Some(MissionCapability::SimulatorTime),
            Self::Signal(condition) => condition.required_capability(),
        }
    }
}

impl NavigationCondition {
    fn validate(&self, field: &str) -> Result<(), ValidationError> {
        match self {
            Self::Altitude { value_m, .. } => {
                validation::finite(&format!("{field}.condition.value_m"), *value_m)
            }
            _ => Ok(()),
        }
    }
}

impl VehicleCondition {
    const fn required_capability(&self) -> Option<MissionCapability> {
        match self {
            Self::Lifecycle { .. } => Some(MissionCapability::LifecycleState),
            Self::GroundContact { .. } | Self::Crashed { .. } => {
                Some(MissionCapability::ContactState)
            }
            Self::LinkValid { .. } | Self::EstimatorValid { .. } => None,
        }
    }
}

impl SignalCondition {
    fn validate(&self, field: &str) -> Result<(), ValidationError> {
        let Self::Value {
            selector,
            comparison,
            value,
        } = self;
        selector.validate(&format!("{field}.condition.selector"))?;
        let value_field = format!("{field}.condition.value");
        match comparison {
            Comparison::AbsoluteLessOrEqual | Comparison::AbsoluteGreaterOrEqual => {
                validation::range(&value_field, *value, 0.0, f64::MAX)
            }
            _ => validation::finite(&value_field, *value),
        }
    }

    const fn required_capability(&self) -> Option<MissionCapability> {
        let Self::Value { selector, .. } = self;
        selector.required_capability()
    }
}
