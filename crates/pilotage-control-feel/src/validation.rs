//! Control-feel profile validation.

use thiserror::Error;

use crate::profile::{AxisDynamics, AxisResponse, FlightFeelProfile, SCHEMA_VERSION};

const MAX_PROFILE_ID_BYTES: usize = 64;
const MAX_DYNAMIC_LIMIT: f32 = 100_000.0;

/// A control-feel profile validation error.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    /// The schema version is not supported.
    #[error("unsupported control-feel schema version {found}")]
    UnsupportedSchema {
        /// Version in the candidate.
        found: u16,
    },
    /// The profile name is empty, too long, or not portable ASCII.
    #[error("invalid control-feel profile_id")]
    InvalidProfileId,
    /// A numeric field is not finite or is outside its permitted range.
    #[error("control-feel field {field} is outside its permitted range")]
    FieldOutOfRange {
        /// Dotted field name.
        field: &'static str,
    },
    /// Two fields have an invalid order.
    #[error("control-feel fields {lower} and {upper} have an invalid order")]
    InvalidOrder {
        /// Field that must not be greater.
        lower: &'static str,
        /// Field that must not be smaller.
        upper: &'static str,
    },
}

/// A serialized control-feel profile load error.
#[derive(Debug, Error)]
pub enum ProfileLoadError {
    /// The JSON cannot be decoded into the strict schema.
    #[error("cannot parse the control-feel profile")]
    Parse(#[source] serde_json::Error),
    /// The decoded profile violates a semantic rule.
    #[error("invalid control-feel profile")]
    Validation(#[from] ValidationError),
}

/// A validated control-feel profile.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedFlightFeelProfile(FlightFeelProfile);

impl ValidatedFlightFeelProfile {
    /// Validate and wrap a profile.
    ///
    /// # Errors
    ///
    /// Returns the first invalid field or cross-field rule.
    pub fn new(profile: FlightFeelProfile) -> Result<Self, ValidationError> {
        validate_identity(&profile)?;
        validate_envelope(&profile)?;
        validate_axis("horizontal", profile.horizontal)?;
        validate_axis("vertical", profile.vertical)?;
        validate_axis("yaw", profile.yaw)?;
        validate_direct(&profile)?;
        validate_hold(&profile)?;
        Ok(Self(profile))
    }

    /// Parse and validate a JSON profile.
    ///
    /// # Errors
    ///
    /// Returns a parse error or the first semantic validation error.
    pub fn from_json_str(text: &str) -> Result<Self, ProfileLoadError> {
        let profile = serde_json::from_str(text).map_err(ProfileLoadError::Parse)?;
        Self::new(profile).map_err(ProfileLoadError::Validation)
    }

    /// Borrow the validated profile.
    #[must_use]
    pub fn profile(&self) -> &FlightFeelProfile {
        &self.0
    }

    /// Consume the wrapper and return the profile.
    #[must_use]
    pub fn into_profile(self) -> FlightFeelProfile {
        self.0
    }
}

impl TryFrom<FlightFeelProfile> for ValidatedFlightFeelProfile {
    type Error = ValidationError;

    fn try_from(value: FlightFeelProfile) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

fn validate_identity(profile: &FlightFeelProfile) -> Result<(), ValidationError> {
    if profile.schema_version != SCHEMA_VERSION {
        return Err(ValidationError::UnsupportedSchema {
            found: profile.schema_version,
        });
    }
    let id = profile.profile_id.as_bytes();
    if id.is_empty()
        || id.len() > MAX_PROFILE_ID_BYTES
        || !id
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_' | b'.'))
    {
        return Err(ValidationError::InvalidProfileId);
    }
    Ok(())
}

fn validate_envelope(profile: &FlightFeelProfile) -> Result<(), ValidationError> {
    let envelope = profile.envelope;
    for (field, value) in [
        (
            "envelope.horizontal_speed_mps",
            envelope.horizontal_speed_mps,
        ),
        ("envelope.vertical_speed_mps", envelope.vertical_speed_mps),
        ("envelope.yaw_rate_rps", envelope.yaw_rate_rps),
        ("envelope.direct_tilt_rad", envelope.direct_tilt_rad),
    ] {
        positive_bounded(field, value, 100.0)?;
    }
    unit_interval("envelope.direct_hover_thrust", envelope.direct_hover_thrust)?;
    unit_interval("envelope.direct_min_thrust", envelope.direct_min_thrust)?;
    unit_interval("envelope.takeoff_input", envelope.takeoff_input)?;
    if envelope.direct_min_thrust > envelope.direct_hover_thrust {
        return Err(ValidationError::InvalidOrder {
            lower: "envelope.direct_min_thrust",
            upper: "envelope.direct_hover_thrust",
        });
    }
    Ok(())
}

fn validate_axis(prefix: &'static str, axis: AxisResponse) -> Result<(), ValidationError> {
    if !axis.curve.expo.is_finite() || !(0.0..=0.8).contains(&axis.curve.expo) {
        return Err(ValidationError::FieldOutOfRange {
            field: match prefix {
                "horizontal" => "horizontal.curve.expo",
                "vertical" => "vertical.curve.expo",
                _ => "yaw.curve.expo",
            },
        });
    }
    if !axis.neutral.active_enter.is_finite()
        || !axis.neutral.active_exit.is_finite()
        || !(0.0..1.0).contains(&axis.neutral.active_enter)
        || !(0.0..1.0).contains(&axis.neutral.active_exit)
        || axis.neutral.active_exit > axis.neutral.active_enter
    {
        return Err(ValidationError::InvalidOrder {
            lower: "axis.neutral.active_exit",
            upper: "axis.neutral.active_enter",
        });
    }
    validate_dynamics(axis.dynamics)
}

fn validate_dynamics(dynamics: AxisDynamics) -> Result<(), ValidationError> {
    for (field, value) in [
        ("axis.dynamics.apply_accel", dynamics.apply_accel),
        ("axis.dynamics.release_accel", dynamics.release_accel),
        ("axis.dynamics.apply_jerk", dynamics.apply_jerk),
        ("axis.dynamics.release_jerk", dynamics.release_jerk),
    ] {
        positive_bounded(field, value, MAX_DYNAMIC_LIMIT)?;
    }
    Ok(())
}

fn validate_direct(profile: &FlightFeelProfile) -> Result<(), ValidationError> {
    let direct = profile.direct;
    for (field, value) in [
        ("direct.tilt_rate_rps", direct.tilt_rate_rps),
        ("direct.tilt_accel_rps2", direct.tilt_accel_rps2),
        ("direct.thrust_rate_per_s", direct.thrust_rate_per_s),
        ("direct.thrust_accel_per_s2", direct.thrust_accel_per_s2),
    ] {
        positive_bounded(field, value, MAX_DYNAMIC_LIMIT)?;
    }
    Ok(())
}

fn validate_hold(profile: &FlightFeelProfile) -> Result<(), ValidationError> {
    positive_bounded("hold.max_speed_mps", profile.hold.max_speed_mps, 10.0)?;
    positive_bounded(
        "hold.max_accel_mps2",
        profile.hold.max_accel_mps2,
        MAX_DYNAMIC_LIMIT,
    )?;
    if profile.hold.stable_dwell_ms > 10_000 {
        return Err(ValidationError::FieldOutOfRange {
            field: "hold.stable_dwell_ms",
        });
    }
    Ok(())
}

fn positive_bounded(field: &'static str, value: f32, upper: f32) -> Result<(), ValidationError> {
    if !value.is_finite() || value <= 0.0 || value > upper {
        return Err(ValidationError::FieldOutOfRange { field });
    }
    Ok(())
}

fn unit_interval(field: &'static str, value: f32) -> Result<(), ValidationError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(ValidationError::FieldOutOfRange { field });
    }
    Ok(())
}
