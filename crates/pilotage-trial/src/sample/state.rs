//! Vehicle, actuator, and environment state values.

use serde::{Deserialize, Serialize};

use crate::{
    MAX_ACTUATOR_VALUES, MAX_CONDITION_VALUES, MAX_TEXT_BYTES, ValidationError,
    validation::{count, finite, nonempty_count, optional_text, text},
};

use super::Observed;

/// A three-component vector.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vector3 {
    /// The first component.
    pub x: f64,
    /// The second component.
    pub y: f64,
    /// The third component.
    pub z: f64,
}

impl Vector3 {
    pub(crate) fn validate(&self, field: &str) -> Result<(), ValidationError> {
        finite(&format!("{field}.x"), self.x)?;
        finite(&format!("{field}.y"), self.y)?;
        finite(&format!("{field}.z"), self.z)
    }
}

/// A scalar-first attitude quaternion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quaternion {
    /// The scalar component.
    pub w: f64,
    /// The first vector component.
    pub x: f64,
    /// The second vector component.
    pub y: f64,
    /// The third vector component.
    pub z: f64,
}

impl Quaternion {
    pub(crate) fn validate(&self, field: &str) -> Result<(), ValidationError> {
        finite(&format!("{field}.w"), self.w)?;
        finite(&format!("{field}.x"), self.x)?;
        finite(&format!("{field}.y"), self.y)?;
        finite(&format!("{field}.z"), self.z)
    }
}

/// Kinematic state in the local north-east-down and body frames.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KinematicState {
    /// The local north-east-down position in meters.
    pub position_m: Observed<Vector3>,
    /// The local north-east-down velocity in meters per second.
    pub velocity_mps: Observed<Vector3>,
    /// The local north-east-down acceleration in meters per second squared.
    pub acceleration_mps2: Observed<Vector3>,
    /// The body attitude relative to the local north-east-down frame.
    pub attitude: Observed<Quaternion>,
    /// The body rates in radians per second.
    pub body_rates_rad_s: Observed<Vector3>,
}

impl KinematicState {
    pub(crate) fn validate(&self, field: &str) -> Result<(), ValidationError> {
        self.position_m
            .validate_with(&format!("{field}.position_m"), Vector3::validate)?;
        self.velocity_mps
            .validate_with(&format!("{field}.velocity_mps"), Vector3::validate)?;
        self.acceleration_mps2
            .validate_with(&format!("{field}.acceleration_mps2"), Vector3::validate)?;
        self.attitude
            .validate_with(&format!("{field}.attitude"), Quaternion::validate)?;
        self.body_rates_rad_s
            .validate_with(&format!("{field}.body_rates_rad_s"), Vector3::validate)
    }
}

/// Actuator effort and saturation data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActuatorState {
    /// The ordered actuator effort values.
    pub values: Vec<f64>,
    /// The actuator saturation state.
    pub saturated: bool,
}

impl ActuatorState {
    pub(crate) fn validate(&self, field: &str) -> Result<(), ValidationError> {
        nonempty_count(
            &format!("{field}.values"),
            self.values.len(),
            MAX_ACTUATOR_VALUES,
        )?;
        for (index, value) in self.values.iter().enumerate() {
            finite(&format!("{field}.values[{index}]"), *value)?;
        }
        Ok(())
    }
}

/// One named scalar condition value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedValue {
    /// The stable value name.
    pub name: String,
    /// The measured value.
    pub value: f64,
}

/// The measured environmental condition state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConditionState {
    /// The wind velocity in the local north-east-down frame.
    pub wind_velocity_ned_mps: Observed<Vector3>,
    /// The root mean square turbulence velocity.
    pub turbulence_rms_mps: Observed<f64>,
    /// Additional ordered condition values.
    pub values: Vec<NamedValue>,
}

impl ConditionState {
    pub(crate) fn validate(&self, field: &str) -> Result<(), ValidationError> {
        self.wind_velocity_ned_mps
            .validate_with(&format!("{field}.wind_velocity_ned_mps"), Vector3::validate)?;
        self.turbulence_rms_mps.validate_with(
            &format!("{field}.turbulence_rms_mps"),
            |value, value_field| finite(value_field, *value),
        )?;
        count(
            &format!("{field}.values"),
            self.values.len(),
            MAX_CONDITION_VALUES,
        )?;
        validate_named_values(field, &self.values)
    }
}

/// A simulator lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    /// The backend is stopped.
    Stopped,
    /// The backend is resetting the trial.
    Resetting,
    /// The backend is waiting for state convergence.
    Converging,
    /// The backend is ready to start.
    Ready,
    /// The vehicle is armed.
    Armed,
    /// The vehicle is disarmed.
    Disarmed,
    /// The backend is stopping the trial.
    Stopping,
}

/// Lifecycle and safety observations for one sample.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleObservation {
    /// The lifecycle state.
    pub state: LifecycleState,
    /// The ground contact state.
    pub ground_contact: bool,
    /// The crash state.
    pub crashed: bool,
}

impl LifecycleObservation {
    pub(crate) const fn validate(&self, _field: &str) -> Result<(), ValidationError> {
        Ok(())
    }
}

/// A validity state for a link or estimator.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthState {
    /// The validity state.
    pub valid: bool,
    /// Additional bounded validity information.
    pub detail: Option<String>,
}

impl HealthState {
    pub(crate) fn validate(&self, field: &str) -> Result<(), ValidationError> {
        optional_text(
            &format!("{field}.detail"),
            self.detail.as_deref(),
            MAX_TEXT_BYTES,
        )
    }
}

fn validate_named_values(field: &str, values: &[NamedValue]) -> Result<(), ValidationError> {
    for (index, item) in values.iter().enumerate() {
        text(
            &format!("{field}.values[{index}].name"),
            &item.name,
            MAX_TEXT_BYTES,
        )?;
        finite(&format!("{field}.values[{index}].value"), item.value)?;
        if values[..index].iter().any(|prior| prior.name == item.name) {
            return Err(ValidationError::DuplicateItem {
                field: format!("{field}.values.name"),
                index,
            });
        }
    }
    Ok(())
}
