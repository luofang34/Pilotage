//! Numeric rules shared by the Aviate runtime phases.
//!
//! Every value that reaches a vehicle command or an evidence record passes
//! through one of these, so a non-finite intermediate cannot become a
//! commanded target or a scored sample.

use super::AviateRuntimeError;

/// The largest angle the runtime treats as one turn.
const TAU: f64 = std::f64::consts::TAU;

/// Rejects a value that is not finite.
///
/// # Errors
///
/// Returns [`AviateRuntimeError`] when the value is infinite or not a number.
pub fn require_finite(field: &'static str, value: f64) -> Result<f64, AviateRuntimeError> {
    if value.is_finite() {
        return Ok(value);
    }
    Err(AviateRuntimeError::InvalidValue { field })
}

/// Bounds one normalized stimulus value to minus one through plus one.
///
/// # Errors
///
/// Returns [`AviateRuntimeError`] when the value is not finite.
pub fn clamp_normalized(field: &'static str, value: f64) -> Result<f64, AviateRuntimeError> {
    Ok(require_finite(field, value)?.clamp(-1.0, 1.0))
}

/// The signed shortest angle from one heading to another, in radians.
///
/// # Errors
///
/// Returns [`AviateRuntimeError`] when either heading is not finite.
pub fn heading_error_rad(from_rad: f64, to_rad: f64) -> Result<f64, AviateRuntimeError> {
    let difference =
        require_finite("target heading", to_rad)? - require_finite("current heading", from_rad)?;
    let wrapped = difference.rem_euclid(TAU);
    Ok(if wrapped > std::f64::consts::PI {
        wrapped - TAU
    } else {
        wrapped
    })
}

/// The yaw of one scalar-first attitude quaternion, in radians.
///
/// # Errors
///
/// Returns [`AviateRuntimeError`] when a component is not finite.
pub fn yaw_rad(attitude_wxyz: [f64; 4]) -> Result<f64, AviateRuntimeError> {
    let [w, x, y, z] = finite_attitude(attitude_wxyz)?;
    let sin_yaw = 2.0 * z.mul_add(w, x * y);
    let cos_yaw = 1.0 - 2.0 * z.mul_add(z, y * y);
    require_finite("yaw", sin_yaw.atan2(cos_yaw))
}

/// The roll of one scalar-first attitude quaternion, in radians.
///
/// # Errors
///
/// Returns [`AviateRuntimeError`] when a component is not finite.
pub fn roll_rad(attitude_wxyz: [f64; 4]) -> Result<f64, AviateRuntimeError> {
    let [w, x, y, z] = finite_attitude(attitude_wxyz)?;
    let sin_roll = 2.0 * x.mul_add(w, y * z);
    let cos_roll = 1.0 - 2.0 * y.mul_add(y, x * x);
    require_finite("roll", sin_roll.atan2(cos_roll))
}

/// The pitch of one scalar-first attitude quaternion, in radians.
///
/// A quaternion at the pole has no finite arcsine argument, so the value
/// is bounded to the pole rather than becoming a number that is not one.
///
/// # Errors
///
/// Returns [`AviateRuntimeError`] when a component is not finite.
pub fn pitch_rad(attitude_wxyz: [f64; 4]) -> Result<f64, AviateRuntimeError> {
    let [w, x, y, z] = finite_attitude(attitude_wxyz)?;
    let sine = 2.0 * y.mul_add(w, -(z * x));
    require_finite("pitch", sine.clamp(-1.0, 1.0).asin())
}

fn finite_attitude(attitude_wxyz: [f64; 4]) -> Result<[f64; 4], AviateRuntimeError> {
    for value in attitude_wxyz {
        require_finite("attitude component", value)?;
    }
    Ok(attitude_wxyz)
}

/// The Euclidean distance between two north-east-down positions, in meters.
///
/// # Errors
///
/// Returns [`AviateRuntimeError`] when a component is not finite.
pub fn distance_m(left: [f64; 3], right: [f64; 3]) -> Result<f64, AviateRuntimeError> {
    let mut total = 0.0;
    for index in 0..3 {
        let difference = require_finite("position component", left[index])?
            - require_finite("position component", right[index])?;
        total = difference.mul_add(difference, total);
    }
    require_finite("position distance", total.sqrt())
}

/// The magnitude of one north-east-down vector.
///
/// # Errors
///
/// Returns [`AviateRuntimeError`] when a component is not finite.
pub fn magnitude(vector: [f64; 3]) -> Result<f64, AviateRuntimeError> {
    distance_m(vector, [0.0; 3])
}

/// The fraction of one span that a value has covered, bounded to zero and one.
///
/// A zero span is a completed span, so a caller cannot divide by it.
#[must_use]
pub fn progress(elapsed_ns: u64, span_ns: u64) -> f64 {
    if span_ns == 0 || elapsed_ns >= span_ns {
        return 1.0;
    }
    elapsed_ns as f64 / span_ns as f64
}

/// Converts whole nanoseconds to seconds.
#[must_use]
pub fn seconds(nanoseconds: u64) -> f64 {
    nanoseconds as f64 / 1_000_000_000.0
}
