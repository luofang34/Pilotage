//! What an attitude does after the stimulus is released.
//!
//! A return-to-zero test asks the vehicle to come back to the angle it left,
//! and the two ways it can answer badly are coming back too far and not
//! stopping. The first is the excursion past the return target on the side the
//! vehicle did not depart from; the second is body-rate activity that is still
//! running when the trial ends.
//!
//! Both are measured against the return target the run resolved, and the
//! excursion uses the shortest signed arc, so a return across the wrap is one
//! small excursion rather than a full turn.

use serde::{Deserialize, Serialize};

use crate::angular::{MINIMUM_ANGULAR_AMPLITUDE_RAD, shortest_arc_rad};
use crate::series::{
    integral_square_linear, timed_window, validate_event, validate_metric_results,
    validate_timed_values, validate_values,
};
use crate::{MetricError, TimedValue};

/// The final interval over which body-rate activity is summarized.
pub const FINAL_RATE_WINDOW_S: f64 = 1.0;

/// The release event and the run-resolved angles for one return to trim.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AngularReleaseSpec {
    /// The release event time, in seconds.
    pub release_time_s: f64,
    /// The angle the vehicle held before it returned, in radians.
    pub departure_rad: f64,
    /// The angle the vehicle returns to, in radians.
    pub return_target_rad: f64,
}

/// Continuous metrics for one angular return to trim.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AngularReleaseMetrics {
    /// The largest excursion past the return target away from the departure
    /// side, in radians.
    pub opposite_return_peak_rad: f64,
    /// The time-weighted root mean square body rate in the final window, in
    /// radians per second.
    pub final_body_rate_rms_rps: f64,
}

/// Calculates return-to-trim metrics from saved angle and body-rate series.
///
/// The body-rate series carries the body-rate magnitude, so a vehicle that
/// swaps activity between two axes cannot report a quiet trial.
///
/// # Errors
///
/// Returns [`MetricError`] when a series is invalid, when the release event is
/// outside a series, or when the departure angle equals the return target.
pub fn measure_angular_release(
    attitude: &[TimedValue],
    body_rate_magnitude: &[TimedValue],
    spec: AngularReleaseSpec,
) -> Result<AngularReleaseMetrics, MetricError> {
    let departure_arc = validate_inputs(attitude, body_rate_magnitude, spec)?;
    let metrics = AngularReleaseMetrics {
        opposite_return_peak_rad: opposite_peak(attitude, spec, departure_arc),
        final_body_rate_rms_rps: final_rate_rms(body_rate_magnitude)?,
    };
    validate_metric_results(&[
        (
            "angular_release.opposite_return_peak_rad",
            metrics.opposite_return_peak_rad,
        ),
        (
            "angular_release.final_body_rate_rms_rps",
            metrics.final_body_rate_rms_rps,
        ),
    ])?;
    Ok(metrics)
}

fn validate_inputs(
    attitude: &[TimedValue],
    body_rate_magnitude: &[TimedValue],
    spec: AngularReleaseSpec,
) -> Result<f64, MetricError> {
    validate_timed_values(attitude)?;
    validate_timed_values(body_rate_magnitude)?;
    if body_rate_magnitude.iter().any(|sample| sample.value < 0.0) {
        return Err(MetricError::InvalidParameter {
            field: "body_rate_magnitude",
        });
    }
    for (field, value) in [
        ("release_time_s", spec.release_time_s),
        ("departure_rad", spec.departure_rad),
        ("return_target_rad", spec.return_target_rad),
    ] {
        if !value.is_finite() {
            return Err(MetricError::InvalidParameter { field });
        }
    }
    let departure_arc = shortest_arc_rad(spec.departure_rad - spec.return_target_rad);
    if departure_arc.abs() < MINIMUM_ANGULAR_AMPLITUDE_RAD {
        return Err(MetricError::ZeroStep);
    }
    validate_event(attitude, "release", spec.release_time_s, |sample| {
        sample.time_s
    })?;
    Ok(departure_arc)
}

/// The largest arc past the return target on the far side from the departure.
///
/// Dividing by the departure direction makes the value negative exactly while
/// the vehicle is past the target, whichever way it departed, so one
/// comparison covers both directions.
fn opposite_peak(attitude: &[TimedValue], spec: AngularReleaseSpec, departure_arc: f64) -> f64 {
    let end_s = attitude[attitude.len() - 1].time_s;
    let direction = departure_arc.signum();
    timed_window(attitude, spec.release_time_s, end_s)
        .into_iter()
        .map(|sample| -shortest_arc_rad(sample.value - spec.return_target_rad) * direction)
        .fold(0.0_f64, f64::max)
}

fn final_rate_rms(body_rate_magnitude: &[TimedValue]) -> Result<f64, MetricError> {
    let end_s = body_rate_magnitude[body_rate_magnitude.len() - 1].time_s;
    let start_s = (end_s - FINAL_RATE_WINDOW_S).max(body_rate_magnitude[0].time_s);
    if end_s <= start_s {
        return Err(MetricError::InvalidParameter {
            field: "body_rate_magnitude",
        });
    }
    let window = timed_window(body_rate_magnitude, start_s, end_s);
    validate_values(&window, "angular_release.body_rate", |sample| sample.value)?;
    let integral = window
        .windows(2)
        .map(|pair| {
            integral_square_linear(
                pair[0].value,
                pair[1].value,
                pair[1].time_s - pair[0].time_s,
            )
        })
        .sum::<f64>();
    Ok((integral / (end_s - start_s)).max(0.0).sqrt())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests;
