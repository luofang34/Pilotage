//! Step response for an angle that wraps.
//!
//! A linear step response subtracts two values. An angular one cannot: a
//! vehicle that holds 179 degrees and is asked for minus 179 degrees was asked
//! for a two-degree step, and plain subtraction reads it as a 358-degree one.
//! Every difference here is reduced to the shortest signed arc first, so the
//! measured amplitude is the angle the vehicle was actually asked to turn
//! through.
//!
//! The baseline and the target are values the run resolved, not values a
//! scenario declared. A direct attitude stimulus measures from the effective
//! setpoint at stimulus entry, so the amplitude is only known once the run has
//! reached that entry.

use serde::{Deserialize, Serialize};

use crate::series::{
    band_settling_time, crossing_delta, first_crossing, timed_window, validate_event,
    validate_metric_results, validate_optional_metric_results, validate_timed_values,
    validate_values,
};
use crate::{MetricError, TimedValue};

/// The directional step fraction that marks a detected angular response.
pub const ANGULAR_DELAY_FRACTION: f64 = 0.02;
/// The lower directional fraction for angular rise time.
pub const ANGULAR_RISE_LOW_FRACTION: f64 = 0.10;
/// The upper directional fraction for angular rise time.
pub const ANGULAR_RISE_HIGH_FRACTION: f64 = 0.90;
/// The half-width of the settling band around the angular step amplitude.
pub const ANGULAR_SETTLING_FRACTION: f64 = 0.05;
/// The final response interval used for angular steady-state error.
pub const ANGULAR_STEADY_STATE_WINDOW_S: f64 = 0.50;
/// The smallest angular step amplitude that carries a measurable response.
pub const MINIMUM_ANGULAR_AMPLITUDE_RAD: f64 = 1.0e-6;

/// The input event and the run-resolved angles for one angular step.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AngularStepSpec {
    /// The input event time, in seconds.
    pub input_time_s: f64,
    /// The angle the run held when the stimulus entered, in radians.
    pub baseline_rad: f64,
    /// The angle the stimulus requested, in radians.
    pub target_rad: f64,
}

/// Continuous metrics for one angular step response.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AngularStepMetrics {
    /// The shortest signed arc from baseline to target, in radians.
    pub amplitude_rad: f64,
    /// Time from input to 2 percent of the angular step, in seconds.
    pub input_to_response_delay_s: Option<f64>,
    /// Time from 10 percent to 90 percent of the angular step, in seconds.
    pub rise_time_s: Option<f64>,
    /// Time from input to final entry in the 5 percent band, in seconds.
    pub settling_time_s: Option<f64>,
    /// The largest response beyond the target, in radians.
    pub overshoot_rad: f64,
    /// Overshoot divided by the absolute step amplitude.
    pub overshoot_fraction: f64,
    /// The largest response behind the baseline, in radians.
    pub undershoot_rad: f64,
    /// The signed mean angular error in the final fixed window, in radians.
    pub steady_state_error_rad: f64,
}

/// Reduces one angle to the shortest signed arc in minus pi through pi.
///
/// The half-open convention puts exactly minus pi at plus pi, so one arc has
/// one representation and a metric cannot change sign on a rounding step.
#[must_use]
pub fn shortest_arc_rad(radians: f64) -> f64 {
    if !radians.is_finite() {
        return radians;
    }
    let turn = core::f64::consts::TAU;
    let wrapped = radians - turn * (radians / turn).round();
    if wrapped <= -core::f64::consts::PI {
        wrapped + turn
    } else {
        wrapped
    }
}

/// Calculates fixed angular step-response metrics from a saved angle series.
///
/// The function reduces every difference to the shortest signed arc. It uses
/// the first directional threshold crossing and the last entry into the
/// settling band. It returns `None` for a threshold the series never reached.
///
/// # Errors
///
/// Returns [`MetricError`] when the series is invalid, when the input event is
/// outside the series, or when the step amplitude is too small to measure.
pub fn measure_angular_step(
    attitude: &[TimedValue],
    spec: AngularStepSpec,
) -> Result<AngularStepMetrics, MetricError> {
    let amplitude_rad = validate_inputs(attitude, spec)?;
    let progress = progress_window(attitude, spec, amplitude_rad);
    validate_values(&progress, "angular.progress", |sample| sample.value)?;
    let magnitude = amplitude_rad.abs();
    let (peak, trough) = excursions(&progress);
    let error = angular_error(attitude, spec);
    validate_values(&error, "angular.error", |sample| sample.value)?;
    let metrics = AngularStepMetrics {
        amplitude_rad,
        input_to_response_delay_s: first_crossing(&progress, ANGULAR_DELAY_FRACTION)
            .map(|time_s| time_s - spec.input_time_s),
        rise_time_s: crossing_delta(
            &progress,
            ANGULAR_RISE_LOW_FRACTION,
            ANGULAR_RISE_HIGH_FRACTION,
        ),
        settling_time_s: band_settling_time(&progress, ANGULAR_SETTLING_FRACTION)
            .map(|time_s| time_s - spec.input_time_s),
        overshoot_rad: (peak - 1.0).max(0.0) * magnitude,
        overshoot_fraction: (peak - 1.0).max(0.0),
        undershoot_rad: (-trough).max(0.0) * magnitude,
        steady_state_error_rad: steady_state_error(&error),
    };
    validate_results(metrics)
}

fn validate_inputs(attitude: &[TimedValue], spec: AngularStepSpec) -> Result<f64, MetricError> {
    validate_timed_values(attitude)?;
    for (field, value) in [
        ("input_time_s", spec.input_time_s),
        ("baseline_rad", spec.baseline_rad),
        ("target_rad", spec.target_rad),
    ] {
        if !value.is_finite() {
            return Err(MetricError::InvalidParameter { field });
        }
    }
    let amplitude_rad = shortest_arc_rad(spec.target_rad - spec.baseline_rad);
    if amplitude_rad.abs() < MINIMUM_ANGULAR_AMPLITUDE_RAD {
        return Err(MetricError::ZeroStep);
    }
    validate_event(attitude, "input", spec.input_time_s, |sample| sample.time_s)?;
    if spec.input_time_s >= attitude[attitude.len() - 1].time_s {
        return Err(MetricError::InvalidParameter {
            field: "input_time_s",
        });
    }
    Ok(amplitude_rad)
}

/// The response as a fraction of the requested arc, from the input onward.
///
/// The direction divides out, so the series rises toward one whether the step
/// was commanded clockwise or counterclockwise, and every threshold reads the
/// same way for both.
fn progress_window(
    attitude: &[TimedValue],
    spec: AngularStepSpec,
    amplitude_rad: f64,
) -> Vec<TimedValue> {
    let end_s = attitude[attitude.len() - 1].time_s;
    timed_window(attitude, spec.input_time_s, end_s)
        .into_iter()
        .map(|sample| TimedValue {
            time_s: sample.time_s,
            value: shortest_arc_rad(sample.value - spec.baseline_rad) / amplitude_rad,
        })
        .collect()
}

fn angular_error(attitude: &[TimedValue], spec: AngularStepSpec) -> Vec<TimedValue> {
    let end_s = attitude[attitude.len() - 1].time_s;
    timed_window(attitude, spec.input_time_s, end_s)
        .into_iter()
        .map(|sample| TimedValue {
            time_s: sample.time_s,
            value: shortest_arc_rad(sample.value - spec.target_rad),
        })
        .collect()
}

fn excursions(progress: &[TimedValue]) -> (f64, f64) {
    let peak = progress
        .iter()
        .map(|sample| sample.value)
        .fold(f64::NEG_INFINITY, f64::max);
    let trough = progress
        .iter()
        .map(|sample| sample.value)
        .fold(f64::INFINITY, f64::min);
    (peak, trough)
}

fn steady_state_error(error: &[TimedValue]) -> f64 {
    let end_s = error[error.len() - 1].time_s;
    let start_s = (end_s - ANGULAR_STEADY_STATE_WINDOW_S).max(error[0].time_s);
    let window = timed_window(error, start_s, end_s);
    if end_s <= start_s {
        return error[error.len() - 1].value;
    }
    let integral = window
        .windows(2)
        .map(|pair| (pair[0].value + pair[1].value) * (pair[1].time_s - pair[0].time_s) / 2.0)
        .sum::<f64>();
    integral / (end_s - start_s)
}

fn validate_results(metrics: AngularStepMetrics) -> Result<AngularStepMetrics, MetricError> {
    validate_optional_metric_results(&[
        (
            "angular.input_to_response_delay_s",
            metrics.input_to_response_delay_s,
        ),
        ("angular.rise_time_s", metrics.rise_time_s),
        ("angular.settling_time_s", metrics.settling_time_s),
    ])?;
    validate_metric_results(&[
        ("angular.amplitude_rad", metrics.amplitude_rad),
        ("angular.overshoot_rad", metrics.overshoot_rad),
        ("angular.overshoot_fraction", metrics.overshoot_fraction),
        ("angular.undershoot_rad", metrics.undershoot_rad),
        (
            "angular.steady_state_error_rad",
            metrics.steady_state_error_rad,
        ),
    ])?;
    Ok(metrics)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests;
