use crate::{EvaluatorError, TelemetrySample};

/// One stable simulator-independent telemetry field.
///
/// A Boolean field uses `0.0` for false and `1.0` for true. A magnitude field
/// is nonnegative. The command and response fields use the units in the
/// scenario plan. [`crate::TelemetrySample::elapsed_ms`] supplies time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CanonicalTelemetryKey {
    /// Boolean crash detection.
    CrashDetected,
    /// Boolean unexpected-contact detection.
    UnexpectedContact,
    /// Position-error magnitude, in meters.
    PositionErrorM,
    /// Attitude-error magnitude, in radians.
    AttitudeErrorRad,
    /// Body-rate magnitude, in radians per second.
    BodyRateRadS,
    /// Absolute load factor, in g.
    LoadFactorG,
    /// Signed normalized actuator effort in the range from minus one to one.
    ActuatorEffort,
    /// Boolean actuator-saturation state.
    ActuatorSaturated,
    /// Boolean estimator-valid state.
    EstimatorValid,
    /// Boolean command-link-valid state.
    CommandLinkValid,
    /// Boolean recovery state.
    Recovered,
    /// Selected scalar command in scenario units.
    CommandPrimary,
    /// Selected scalar response in the same units as the command.
    ResponsePrimary,
    /// Selected-axis truth position, in meters.
    PositionPrimaryM,
    /// Selected-axis truth velocity, in meters per second.
    VelocityPrimaryMps,
    /// Selected-axis truth acceleration, in meters per second squared.
    AccelerationPrimaryMps2,
    /// Wind-speed magnitude, in meters per second.
    WindSpeedMps,
    /// Position-error magnitude during wind, in meters.
    WindPositionErrorM,
}

impl CanonicalTelemetryKey {
    /// All required canonical fields in stable order.
    pub const ALL: [Self; 18] = [
        Self::CrashDetected,
        Self::UnexpectedContact,
        Self::PositionErrorM,
        Self::AttitudeErrorRad,
        Self::BodyRateRadS,
        Self::LoadFactorG,
        Self::ActuatorEffort,
        Self::ActuatorSaturated,
        Self::EstimatorValid,
        Self::CommandLinkValid,
        Self::Recovered,
        Self::CommandPrimary,
        Self::ResponsePrimary,
        Self::PositionPrimaryM,
        Self::VelocityPrimaryMps,
        Self::AccelerationPrimaryMps2,
        Self::WindSpeedMps,
        Self::WindPositionErrorM,
    ];

    /// Returns the exact telemetry map key.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CrashDetected => "safety.crash_detected",
            Self::UnexpectedContact => "safety.unexpected_contact",
            Self::PositionErrorM => "tracking.position_error_m",
            Self::AttitudeErrorRad => "tracking.attitude_error_rad",
            Self::BodyRateRadS => "motion.body_rate_rad_s",
            Self::LoadFactorG => "motion.load_factor_g",
            Self::ActuatorEffort => "control.actuator_effort",
            Self::ActuatorSaturated => "control.actuator_saturated",
            Self::EstimatorValid => "health.estimator_valid",
            Self::CommandLinkValid => "health.command_link_valid",
            Self::Recovered => "safety.recovered",
            Self::CommandPrimary => "response.command_primary",
            Self::ResponsePrimary => "response.value_primary",
            Self::PositionPrimaryM => "motion.position_primary_m",
            Self::VelocityPrimaryMps => "motion.velocity_primary_mps",
            Self::AccelerationPrimaryMps2 => "motion.acceleration_primary_mps2",
            Self::WindSpeedMps => "environment.wind_speed_mps",
            Self::WindPositionErrorM => "tracking.wind_position_error_m",
        }
    }
}

pub(super) struct CanonicalSample {
    pub(super) time_s: f64,
    pub(super) command: f64,
    pub(super) response: f64,
    pub(super) position_m: f64,
    pub(super) velocity_mps: f64,
    pub(super) acceleration_mps2: f64,
    pub(super) effort: f64,
    pub(super) saturated: bool,
    pub(super) wind_speed_mps: f64,
    pub(super) wind_position_error_m: f64,
}

impl CanonicalSample {
    pub(super) fn decode(sample: &TelemetrySample) -> Result<Self, EvaluatorError> {
        for key in CanonicalTelemetryKey::ALL {
            finite_value(sample, key)?;
        }
        Ok(Self {
            time_s: sample.elapsed_ms as f64 / 1_000.0,
            command: finite_value(sample, CanonicalTelemetryKey::CommandPrimary)?,
            response: finite_value(sample, CanonicalTelemetryKey::ResponsePrimary)?,
            position_m: finite_value(sample, CanonicalTelemetryKey::PositionPrimaryM)?,
            velocity_mps: finite_value(sample, CanonicalTelemetryKey::VelocityPrimaryMps)?,
            acceleration_mps2: finite_value(
                sample,
                CanonicalTelemetryKey::AccelerationPrimaryMps2,
            )?,
            effort: finite_value(sample, CanonicalTelemetryKey::ActuatorEffort)?,
            saturated: flag(sample, CanonicalTelemetryKey::ActuatorSaturated)?,
            wind_speed_mps: finite_value(sample, CanonicalTelemetryKey::WindSpeedMps)?,
            wind_position_error_m: finite_value(sample, CanonicalTelemetryKey::WindPositionErrorM)?,
        })
    }
}

pub(super) fn finite_value(
    sample: &TelemetrySample,
    key: CanonicalTelemetryKey,
) -> Result<f64, EvaluatorError> {
    let value = sample
        .values
        .get(key.as_str())
        .copied()
        .ok_or_else(|| invalid(format!("sample has no canonical field {}", key.as_str())))?;
    if !value.is_finite() {
        return Err(invalid(format!(
            "canonical field {} is not finite",
            key.as_str()
        )));
    }
    Ok(value)
}

pub(super) fn flag(
    sample: &TelemetrySample,
    key: CanonicalTelemetryKey,
) -> Result<bool, EvaluatorError> {
    match finite_value(sample, key)? {
        0.0 => Ok(false),
        1.0 => Ok(true),
        _ => Err(invalid(format!(
            "canonical field {} is not a Boolean value",
            key.as_str()
        ))),
    }
}

fn invalid(detail: impl Into<String>) -> EvaluatorError {
    EvaluatorError::new(detail)
}
