//! Control values at each stage of the sample path.

use serde::{Deserialize, Serialize};

use crate::{
    MAX_ACTUATOR_VALUES, MAX_RAW_AXES, MAX_RAW_BUTTONS, MAX_TEXT_BYTES, ValidationError,
    validation::{count, finite, nonempty_count, optional_text, range},
};

use super::{Quaternion, Vector3};

/// A coordinate frame for a control value.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceFrame {
    /// The local north-east-down frame.
    LocalNed,
    /// The body forward-right-down frame.
    BodyFrd,
}

/// Four normalized input channels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlAxes {
    /// The normalized roll value.
    pub roll: f64,
    /// The normalized pitch value.
    pub pitch: f64,
    /// The normalized vertical value.
    pub vertical: f64,
    /// The normalized yaw value.
    pub yaw: f64,
}

impl ControlAxes {
    pub(crate) fn validate(&self, field: &str) -> Result<(), ValidationError> {
        range(&format!("{field}.roll"), self.roll, -1.0, 1.0)?;
        range(&format!("{field}.pitch"), self.pitch, -1.0, 1.0)?;
        range(&format!("{field}.vertical"), self.vertical, -1.0, 1.0)?;
        range(&format!("{field}.yaw"), self.yaw, -1.0, 1.0)
    }
}

/// One raw device input report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawInput {
    /// The device axis values in profile order.
    pub axes: Vec<f64>,
    /// The device button values in profile order.
    pub buttons: Vec<bool>,
}

impl RawInput {
    pub(crate) fn validate(&self, field: &str) -> Result<(), ValidationError> {
        count(&format!("{field}.axes"), self.axes.len(), MAX_RAW_AXES)?;
        count(
            &format!("{field}.buttons"),
            self.buttons.len(),
            MAX_RAW_BUTTONS,
        )?;
        for (index, value) in self.axes.iter().enumerate() {
            finite(&format!("{field}.axes[{index}]"), *value)?;
        }
        Ok(())
    }
}

/// A typed control value at one pipeline stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlValue {
    /// Four normalized control axes.
    Axes {
        /// The normalized control axes.
        axes: ControlAxes,
    },
    /// Linear velocity and yaw rate.
    Velocity {
        /// The coordinate frame.
        frame: ReferenceFrame,
        /// The linear velocity in meters per second.
        linear_mps: Vector3,
        /// The yaw rate in radians per second.
        yaw_rate_rad_s: f64,
    },
    /// Attitude and normalized thrust.
    AttitudeThrust {
        /// The coordinate frame.
        frame: ReferenceFrame,
        /// The requested attitude.
        attitude: Quaternion,
        /// The normalized thrust from zero through one.
        thrust: f64,
    },
    /// Body rate and normalized thrust.
    BodyRateThrust {
        /// The body rate in radians per second.
        body_rates_rad_s: Vector3,
        /// The normalized thrust from zero through one.
        thrust: f64,
    },
    /// Position and yaw angle.
    PositionYaw {
        /// The coordinate frame.
        frame: ReferenceFrame,
        /// The position in meters.
        position_m: Vector3,
        /// The yaw angle in radians.
        yaw_rad: f64,
    },
    /// Ordered scalar channels for an adapter-specific control type.
    ScalarChannels {
        /// The ordered channel values.
        values: Vec<f64>,
    },
}

impl ControlValue {
    pub(crate) fn validate(&self, field: &str) -> Result<(), ValidationError> {
        match self {
            Self::Axes { axes } => axes.validate(field),
            Self::Velocity {
                linear_mps,
                yaw_rate_rad_s,
                ..
            } => {
                linear_mps.validate(&format!("{field}.linear_mps"))?;
                finite(&format!("{field}.yaw_rate_rad_s"), *yaw_rate_rad_s)
            }
            Self::AttitudeThrust {
                attitude, thrust, ..
            } => {
                attitude.validate(&format!("{field}.attitude"))?;
                range(&format!("{field}.thrust"), *thrust, 0.0, 1.0)
            }
            Self::BodyRateThrust {
                body_rates_rad_s,
                thrust,
            } => {
                body_rates_rad_s.validate(&format!("{field}.body_rates_rad_s"))?;
                range(&format!("{field}.thrust"), *thrust, 0.0, 1.0)
            }
            Self::PositionYaw {
                position_m,
                yaw_rad,
                ..
            } => {
                position_m.validate(&format!("{field}.position_m"))?;
                finite(&format!("{field}.yaw_rad"), *yaw_rad)
            }
            Self::ScalarChannels { values } => validate_scalars(field, values),
        }
    }
}

/// The adapter decision for one control demand.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AdapterDisposition {
    /// The adapter accepted the demand without a constraint.
    Accepted,
    /// The adapter constrained the demand.
    Constrained {
        /// The constraint reason.
        reason: Option<String>,
    },
    /// The adapter rejected the demand.
    Rejected {
        /// The rejection reason.
        reason: Option<String>,
    },
}

impl AdapterDisposition {
    pub(crate) fn validate(&self, field: &str) -> Result<(), ValidationError> {
        match self {
            Self::Accepted => Ok(()),
            Self::Constrained { reason } | Self::Rejected { reason } => optional_text(
                &format!("{field}.reason"),
                reason.as_deref(),
                MAX_TEXT_BYTES,
            ),
        }
    }
}

fn validate_scalars(field: &str, values: &[f64]) -> Result<(), ValidationError> {
    nonempty_count(
        &format!("{field}.values"),
        values.len(),
        MAX_ACTUATOR_VALUES,
    )?;
    for (index, value) in values.iter().enumerate() {
        finite(&format!("{field}.values[{index}]"), *value)?;
    }
    Ok(())
}
