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
        validate_bindings(&profile)?;
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

fn validate_bindings(profile: &FlightFeelProfile) -> Result<(), ValidationError> {
    if profile.bindings.device_profile_sha256.as_bytes() == &[0_u8; 32] {
        return Err(ValidationError::FieldOutOfRange {
            field: "bindings.device_profile_sha256",
        });
    }
    if profile.bindings.flight_controller_sha256.as_bytes() == &[0_u8; 32] {
        return Err(ValidationError::FieldOutOfRange {
            field: "bindings.flight_controller_sha256",
        });
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
    if envelope.takeoff_input >= 1.0 {
        return Err(ValidationError::FieldOutOfRange {
            field: "envelope.takeoff_input",
        });
    }
    if envelope.direct_min_thrust > envelope.direct_hover_thrust {
        return Err(ValidationError::InvalidOrder {
            lower: "envelope.direct_min_thrust",
            upper: "envelope.direct_hover_thrust",
        });
    }
    Ok(())
}

fn validate_axis(prefix: &'static str, axis: AxisResponse) -> Result<(), ValidationError> {
    for (suffix, value, inclusive_upper) in [
        ("curve.deadzone", axis.curve.deadzone, false),
        ("curve.center_expo", axis.curve.center_expo, true),
        ("curve.outer_expo", axis.curve.outer_expo, true),
        ("curve.outer_start", axis.curve.outer_start, true),
    ] {
        let valid = value.is_finite()
            && value >= 0.0
            && if inclusive_upper {
                value <= 1.0
            } else {
                value < 1.0
            };
        if !valid {
            return Err(ValidationError::FieldOutOfRange {
                field: axis_field(prefix, suffix),
            });
        }
    }
    if axis.curve.outer_expo > axis.curve.center_expo {
        return Err(ValidationError::InvalidOrder {
            lower: axis_field(prefix, "curve.outer_expo"),
            upper: axis_field(prefix, "curve.center_expo"),
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
    if axis.neutral.dwell_ms > 10_000 {
        return Err(ValidationError::FieldOutOfRange {
            field: "axis.neutral.dwell_ms",
        });
    }
    validate_dynamics(axis.dynamics)
}

fn axis_field(prefix: &'static str, suffix: &'static str) -> &'static str {
    match (prefix, suffix) {
        ("horizontal", "curve.deadzone") => "horizontal.curve.deadzone",
        ("horizontal", "curve.center_expo") => "horizontal.curve.center_expo",
        ("horizontal", "curve.outer_expo") => "horizontal.curve.outer_expo",
        ("horizontal", _) => "horizontal.curve.outer_start",
        ("vertical", "curve.deadzone") => "vertical.curve.deadzone",
        ("vertical", "curve.center_expo") => "vertical.curve.center_expo",
        ("vertical", "curve.outer_expo") => "vertical.curve.outer_expo",
        ("vertical", _) => "vertical.curve.outer_start",
        ("yaw", "curve.deadzone") => "yaw.curve.deadzone",
        ("yaw", "curve.center_expo") => "yaw.curve.center_expo",
        ("yaw", "curve.outer_expo") => "yaw.curve.outer_expo",
        _ => "yaw.curve.outer_start",
    }
}

fn validate_dynamics(dynamics: AxisDynamics) -> Result<(), ValidationError> {
    for (field, value) in [
        ("axis.dynamics.apply_accel", dynamics.apply_accel),
        ("axis.dynamics.release_accel", dynamics.release_accel),
        ("axis.dynamics.apply_jerk", dynamics.apply_jerk),
        ("axis.dynamics.release_jerk", dynamics.release_jerk),
        ("axis.dynamics.reversal_accel", dynamics.reversal_accel),
        ("axis.dynamics.reversal_jerk", dynamics.reversal_jerk),
    ] {
        positive_bounded(field, value, MAX_DYNAMIC_LIMIT)?;
    }
    if dynamics.release_accel < dynamics.apply_accel {
        return Err(ValidationError::InvalidOrder {
            lower: "axis.dynamics.apply_accel",
            upper: "axis.dynamics.release_accel",
        });
    }
    if dynamics.release_jerk < dynamics.apply_jerk {
        return Err(ValidationError::InvalidOrder {
            lower: "axis.dynamics.apply_jerk",
            upper: "axis.dynamics.release_jerk",
        });
    }
    if dynamics.reversal_accel > dynamics.release_accel {
        return Err(ValidationError::InvalidOrder {
            lower: "axis.dynamics.reversal_accel",
            upper: "axis.dynamics.release_accel",
        });
    }
    if dynamics.reversal_jerk > dynamics.release_jerk {
        return Err(ValidationError::InvalidOrder {
            lower: "axis.dynamics.reversal_jerk",
            upper: "axis.dynamics.release_jerk",
        });
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
