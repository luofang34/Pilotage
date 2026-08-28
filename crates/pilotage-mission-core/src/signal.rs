//! Exact scalar selectors for signal conditions.

use serde::{Deserialize, Serialize};

use crate::{MissionCapability, ValidationError, validation};

const MAX_RAW_AXES: usize = 64;
const MAX_ACTUATOR_VALUES: usize = 256;

/// A control channel for a trial stimulus or normalized input.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
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

/// A component of a three-value vector.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum VectorComponent {
    /// The first component.
    X,
    /// The second component.
    Y,
    /// The third component.
    Z,
}

/// A component of a quaternion.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum QuaternionComponent {
    /// The scalar component.
    W,
    /// The first vector component.
    X,
    /// The second vector component.
    Y,
    /// The third vector component.
    Z,
}

/// A reference frame for a typed control value.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ReferenceFrame {
    /// A local north-east-down frame.
    LocalNed,
    /// A body forward-right-down frame.
    BodyFrd,
}

/// One exact field of a tagged control value.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlValueField {
    /// The roll field of an axes value.
    AxisRoll {},
    /// The pitch field of an axes value.
    AxisPitch {},
    /// The vertical field of an axes value.
    AxisVertical {},
    /// The yaw field of an axes value.
    AxisYaw {},
    /// The first linear field of a velocity value.
    VelocityX {
        /// The reference frame that the value must use.
        expected_frame: ReferenceFrame,
    },
    /// The second linear field of a velocity value.
    VelocityY {
        /// The reference frame that the value must use.
        expected_frame: ReferenceFrame,
    },
    /// The third linear field of a velocity value.
    VelocityZ {
        /// The reference frame that the value must use.
        expected_frame: ReferenceFrame,
    },
    /// The yaw-rate field of a velocity value.
    VelocityYawRate {
        /// The reference frame that the value must use.
        expected_frame: ReferenceFrame,
    },
    /// The scalar field of an attitude-thrust value.
    AttitudeW {
        /// The reference frame that the value must use.
        expected_frame: ReferenceFrame,
    },
    /// The first vector field of an attitude-thrust value.
    AttitudeX {
        /// The reference frame that the value must use.
        expected_frame: ReferenceFrame,
    },
    /// The second vector field of an attitude-thrust value.
    AttitudeY {
        /// The reference frame that the value must use.
        expected_frame: ReferenceFrame,
    },
    /// The third vector field of an attitude-thrust value.
    AttitudeZ {
        /// The reference frame that the value must use.
        expected_frame: ReferenceFrame,
    },
    /// The thrust field of an attitude-thrust value.
    AttitudeThrust {
        /// The reference frame that the value must use.
        expected_frame: ReferenceFrame,
    },
    /// The first rate field of a body-rate-thrust value.
    BodyRateX {},
    /// The second rate field of a body-rate-thrust value.
    BodyRateY {},
    /// The third rate field of a body-rate-thrust value.
    BodyRateZ {},
    /// The thrust field of a body-rate-thrust value.
    BodyRateThrust {},
    /// The first position field of a position-yaw value.
    PositionX {
        /// The reference frame that the value must use.
        expected_frame: ReferenceFrame,
    },
    /// The second position field of a position-yaw value.
    PositionY {
        /// The reference frame that the value must use.
        expected_frame: ReferenceFrame,
    },
    /// The third position field of a position-yaw value.
    PositionZ {
        /// The reference frame that the value must use.
        expected_frame: ReferenceFrame,
    },
    /// The yaw field of a position-yaw value.
    PositionYaw {
        /// The reference frame that the value must use.
        expected_frame: ReferenceFrame,
    },
    /// One field of a scalar-channel value.
    ScalarChannel {
        /// The zero-based scalar-channel index.
        index: u16,
    },
}

/// A selector for one exact scalar in a mission observation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum SignalSelector {
    /// One raw input axis.
    RawInputAxis {
        /// The zero-based raw axis index.
        index: u16,
    },
    /// One normalized control channel.
    NormalizedControl {
        /// The selected channel.
        channel: ControlChannel,
    },
    /// One field of the typed intent.
    TypedIntent {
        /// The selected tagged-value field.
        field: ControlValueField,
    },
    /// One field of the adapter demand.
    AdapterDemand {
        /// The selected tagged-value field.
        field: ControlValueField,
    },
    /// One field of the transmitted setpoint.
    TransmittedSetpoint {
        /// The selected tagged-value field.
        field: ControlValueField,
    },
    /// One estimated position component.
    EstimatePosition {
        /// The selected vector component.
        component: VectorComponent,
    },
    /// One estimated velocity component.
    EstimateVelocity {
        /// The selected vector component.
        component: VectorComponent,
    },
    /// One estimated acceleration component.
    EstimateAcceleration {
        /// The selected vector component.
        component: VectorComponent,
    },
    /// One estimated attitude component.
    EstimateAttitude {
        /// The selected quaternion component.
        component: QuaternionComponent,
    },
    /// One estimated body-rate component.
    EstimateBodyRate {
        /// The selected vector component.
        component: VectorComponent,
    },
    /// One truth position component.
    TruthPosition {
        /// The selected vector component.
        component: VectorComponent,
    },
    /// One truth velocity component.
    TruthVelocity {
        /// The selected vector component.
        component: VectorComponent,
    },
    /// One truth acceleration component.
    TruthAcceleration {
        /// The selected vector component.
        component: VectorComponent,
    },
    /// One truth attitude component.
    TruthAttitude {
        /// The selected quaternion component.
        component: QuaternionComponent,
    },
    /// One truth body-rate component.
    TruthBodyRate {
        /// The selected vector component.
        component: VectorComponent,
    },
    /// One actuator value.
    Actuator {
        /// The zero-based actuator index.
        index: u16,
    },
    /// One additional environmental condition value.
    ConditionValue {
        /// The exact condition value name.
        name: String,
    },
}

impl SignalSelector {
    pub(crate) fn validate(&self, field: &str) -> Result<(), ValidationError> {
        match self {
            Self::RawInputAxis { index } => {
                validate_index(&format!("{field}.index"), *index, MAX_RAW_AXES)
            }
            Self::TypedIntent { field: value }
            | Self::AdapterDemand { field: value }
            | Self::TransmittedSetpoint { field: value } => value.validate(field),
            Self::Actuator { index } => {
                validate_index(&format!("{field}.index"), *index, MAX_ACTUATOR_VALUES)
            }
            Self::ConditionValue { name } => validation::text(&format!("{field}.name"), name),
            _ => Ok(()),
        }
    }

    pub(crate) const fn required_capability(&self) -> Option<MissionCapability> {
        match self {
            Self::TruthPosition { .. }
            | Self::TruthVelocity { .. }
            | Self::TruthAcceleration { .. }
            | Self::TruthAttitude { .. }
            | Self::TruthBodyRate { .. } => Some(MissionCapability::KinematicTruth),
            Self::ConditionValue { .. } => Some(MissionCapability::ConditionControl),
            _ => None,
        }
    }
}

impl ControlValueField {
    fn validate(self, field: &str) -> Result<(), ValidationError> {
        match self {
            Self::ScalarChannel { index } => {
                validate_index(&format!("{field}.field.index"), index, MAX_ACTUATOR_VALUES)
            }
            _ => Ok(()),
        }
    }
}

fn validate_index(field: &str, index: u16, limit: usize) -> Result<(), ValidationError> {
    if usize::from(index) < limit {
        return Ok(());
    }
    Err(ValidationError::OutOfRange {
        field: field.to_owned(),
        actual: f64::from(index),
        minimum: 0.0,
        maximum: limit.saturating_sub(1) as f64,
    })
}
