//! Exact scalar selectors for scenario conditions.

mod error;

use serde::{Deserialize, Serialize};

use crate::BackendCapability;
use crate::{
    ControlValue, MAX_ACTUATOR_VALUES, MAX_RAW_AXES, MAX_TEXT_BYTES, ReferenceFrame,
    ValidationError, validation::text,
};

pub use error::SignalSelectionError;

/// A control channel for a stimulus or normalized input.
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

/// A component of a three-value vector.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorComponent {
    /// The first component.
    X,
    /// The second component.
    Y,
    /// The third component.
    Z,
}

/// A component of a quaternion.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

/// One exact field of a tagged control value.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlValueField {
    /// The roll field of an axes value.
    AxisRoll,
    /// The pitch field of an axes value.
    AxisPitch,
    /// The vertical field of an axes value.
    AxisVertical,
    /// The yaw field of an axes value.
    AxisYaw,
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
    /// The scalar field of an attitude-thrust quaternion.
    AttitudeW {
        /// The reference frame that the value must use.
        expected_frame: ReferenceFrame,
    },
    /// The first vector field of an attitude-thrust quaternion.
    AttitudeX {
        /// The reference frame that the value must use.
        expected_frame: ReferenceFrame,
    },
    /// The second vector field of an attitude-thrust quaternion.
    AttitudeY {
        /// The reference frame that the value must use.
        expected_frame: ReferenceFrame,
    },
    /// The third vector field of an attitude-thrust quaternion.
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
    BodyRateX,
    /// The second rate field of a body-rate-thrust value.
    BodyRateY,
    /// The third rate field of a body-rate-thrust value.
    BodyRateZ,
    /// The thrust field of a body-rate-thrust value.
    BodyRateThrust,
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

/// A selector for one exact scalar in a trial sample.
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
    pub(super) fn validate(&self, field: &str) -> Result<(), ValidationError> {
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
            Self::ConditionValue { name } => text(&format!("{field}.name"), name, MAX_TEXT_BYTES),
            _ => Ok(()),
        }
    }

    pub(super) const fn required_capability(&self) -> Option<BackendCapability> {
        match self {
            Self::TruthPosition { .. }
            | Self::TruthVelocity { .. }
            | Self::TruthAcceleration { .. }
            | Self::TruthAttitude { .. }
            | Self::TruthBodyRate { .. } => Some(BackendCapability::KinematicTruth),
            Self::ConditionValue { .. } => Some(BackendCapability::ConditionControl),
            _ => None,
        }
    }
}

impl ControlValueField {
    /// Selects this scalar from one tagged control value.
    ///
    /// # Errors
    ///
    /// Returns an error if the value has a different variant or reference
    /// frame. It also returns an error if a scalar channel is not present.
    pub fn select(&self, value: &ControlValue) -> Result<f64, SignalSelectionError> {
        self.validate_runtime_shape(value)?;
        match (self, value) {
            (Self::AxisRoll, ControlValue::Axes { axes }) => Ok(axes.roll),
            (Self::AxisPitch, ControlValue::Axes { axes }) => Ok(axes.pitch),
            (Self::AxisVertical, ControlValue::Axes { axes }) => Ok(axes.vertical),
            (Self::AxisYaw, ControlValue::Axes { axes }) => Ok(axes.yaw),
            (Self::VelocityX { .. }, ControlValue::Velocity { linear_mps, .. }) => Ok(linear_mps.x),
            (Self::VelocityY { .. }, ControlValue::Velocity { linear_mps, .. }) => Ok(linear_mps.y),
            (Self::VelocityZ { .. }, ControlValue::Velocity { linear_mps, .. }) => Ok(linear_mps.z),
            (Self::VelocityYawRate { .. }, ControlValue::Velocity { yaw_rate_rad_s, .. }) => {
                Ok(*yaw_rate_rad_s)
            }
            (Self::AttitudeW { .. }, ControlValue::AttitudeThrust { attitude, .. }) => {
                Ok(attitude.w)
            }
            (Self::AttitudeX { .. }, ControlValue::AttitudeThrust { attitude, .. }) => {
                Ok(attitude.x)
            }
            (Self::AttitudeY { .. }, ControlValue::AttitudeThrust { attitude, .. }) => {
                Ok(attitude.y)
            }
            (Self::AttitudeZ { .. }, ControlValue::AttitudeThrust { attitude, .. }) => {
                Ok(attitude.z)
            }
            (Self::AttitudeThrust { .. }, ControlValue::AttitudeThrust { thrust, .. }) => {
                Ok(*thrust)
            }
            (
                Self::BodyRateX,
                ControlValue::BodyRateThrust {
                    body_rates_rad_s, ..
                },
            ) => Ok(body_rates_rad_s.x),
            (
                Self::BodyRateY,
                ControlValue::BodyRateThrust {
                    body_rates_rad_s, ..
                },
            ) => Ok(body_rates_rad_s.y),
            (
                Self::BodyRateZ,
                ControlValue::BodyRateThrust {
                    body_rates_rad_s, ..
                },
            ) => Ok(body_rates_rad_s.z),
            (Self::BodyRateThrust, ControlValue::BodyRateThrust { thrust, .. }) => Ok(*thrust),
            (Self::PositionX { .. }, ControlValue::PositionYaw { position_m, .. }) => {
                Ok(position_m.x)
            }
            (Self::PositionY { .. }, ControlValue::PositionYaw { position_m, .. }) => {
                Ok(position_m.y)
            }
            (Self::PositionZ { .. }, ControlValue::PositionYaw { position_m, .. }) => {
                Ok(position_m.z)
            }
            (Self::PositionYaw { .. }, ControlValue::PositionYaw { yaw_rad, .. }) => Ok(*yaw_rad),
            (Self::ScalarChannel { index }, ControlValue::ScalarChannels { values }) => values
                .get(usize::from(*index))
                .copied()
                .ok_or(SignalSelectionError::ScalarChannelUnavailable {
                    index: *index,
                    count: values.len(),
                }),
            _ => Err(self.variant_mismatch(value)),
        }
    }

    fn validate_runtime_shape(&self, value: &ControlValue) -> Result<(), SignalSelectionError> {
        if self.expected_variant() != control_value_variant(value) {
            return Err(self.variant_mismatch(value));
        }
        if let (Some(expected), Some(actual)) = (self.expected_frame(), control_value_frame(value))
            && expected != actual
        {
            return Err(SignalSelectionError::ReferenceFrameMismatch {
                selector: self.name(),
                expected: reference_frame_name(expected),
                actual: reference_frame_name(actual),
            });
        }
        Ok(())
    }

    fn variant_mismatch(&self, value: &ControlValue) -> SignalSelectionError {
        SignalSelectionError::ControlValueVariantMismatch {
            selector: self.name(),
            expected: self.expected_variant(),
            actual: control_value_variant(value),
        }
    }

    const fn expected_variant(&self) -> &'static str {
        match self {
            Self::AxisRoll | Self::AxisPitch | Self::AxisVertical | Self::AxisYaw => "axes",
            Self::VelocityX { .. }
            | Self::VelocityY { .. }
            | Self::VelocityZ { .. }
            | Self::VelocityYawRate { .. } => "velocity",
            Self::AttitudeW { .. }
            | Self::AttitudeX { .. }
            | Self::AttitudeY { .. }
            | Self::AttitudeZ { .. }
            | Self::AttitudeThrust { .. } => "attitude_thrust",
            Self::BodyRateX | Self::BodyRateY | Self::BodyRateZ | Self::BodyRateThrust => {
                "body_rate_thrust"
            }
            Self::PositionX { .. }
            | Self::PositionY { .. }
            | Self::PositionZ { .. }
            | Self::PositionYaw { .. } => "position_yaw",
            Self::ScalarChannel { .. } => "scalar_channels",
        }
    }

    const fn expected_frame(&self) -> Option<ReferenceFrame> {
        match self {
            Self::VelocityX { expected_frame }
            | Self::VelocityY { expected_frame }
            | Self::VelocityZ { expected_frame }
            | Self::VelocityYawRate { expected_frame }
            | Self::AttitudeW { expected_frame }
            | Self::AttitudeX { expected_frame }
            | Self::AttitudeY { expected_frame }
            | Self::AttitudeZ { expected_frame }
            | Self::AttitudeThrust { expected_frame }
            | Self::PositionX { expected_frame }
            | Self::PositionY { expected_frame }
            | Self::PositionZ { expected_frame }
            | Self::PositionYaw { expected_frame } => Some(*expected_frame),
            _ => None,
        }
    }

    const fn name(&self) -> &'static str {
        match self {
            Self::AxisRoll => "axis_roll",
            Self::AxisPitch => "axis_pitch",
            Self::AxisVertical => "axis_vertical",
            Self::AxisYaw => "axis_yaw",
            Self::VelocityX { .. } => "velocity_x",
            Self::VelocityY { .. } => "velocity_y",
            Self::VelocityZ { .. } => "velocity_z",
            Self::VelocityYawRate { .. } => "velocity_yaw_rate",
            Self::AttitudeW { .. } => "attitude_w",
            Self::AttitudeX { .. } => "attitude_x",
            Self::AttitudeY { .. } => "attitude_y",
            Self::AttitudeZ { .. } => "attitude_z",
            Self::AttitudeThrust { .. } => "attitude_thrust",
            Self::BodyRateX => "body_rate_x",
            Self::BodyRateY => "body_rate_y",
            Self::BodyRateZ => "body_rate_z",
            Self::BodyRateThrust => "body_rate_thrust",
            Self::PositionX { .. } => "position_x",
            Self::PositionY { .. } => "position_y",
            Self::PositionZ { .. } => "position_z",
            Self::PositionYaw { .. } => "position_yaw",
            Self::ScalarChannel { .. } => "scalar_channel",
        }
    }

    fn validate(self, field: &str) -> Result<(), ValidationError> {
        match self {
            Self::ScalarChannel { index } => {
                validate_index(&format!("{field}.field.index"), index, MAX_ACTUATOR_VALUES)
            }
            _ => Ok(()),
        }
    }
}

const fn control_value_variant(value: &ControlValue) -> &'static str {
    match value {
        ControlValue::Axes { .. } => "axes",
        ControlValue::Velocity { .. } => "velocity",
        ControlValue::AttitudeThrust { .. } => "attitude_thrust",
        ControlValue::BodyRateThrust { .. } => "body_rate_thrust",
        ControlValue::PositionYaw { .. } => "position_yaw",
        ControlValue::ScalarChannels { .. } => "scalar_channels",
    }
}

const fn control_value_frame(value: &ControlValue) -> Option<ReferenceFrame> {
    match value {
        ControlValue::Velocity { frame, .. }
        | ControlValue::AttitudeThrust { frame, .. }
        | ControlValue::PositionYaw { frame, .. } => Some(*frame),
        _ => None,
    }
}

const fn reference_frame_name(frame: ReferenceFrame) -> &'static str {
    match frame {
        ReferenceFrame::LocalNed => "local_ned",
        ReferenceFrame::BodyFrd => "body_frd",
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
