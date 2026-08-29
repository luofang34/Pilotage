//! Collective force against the acceleration it produced.
//!
//! A collective command is a force, and the response is not another force: it
//! is the vertical acceleration the vehicle answered with. The two are in
//! different units, so nothing here normalizes one against the other. What is
//! measured is whether the acceleration went the way the force asked, how
//! large it was, and how much of the window it spent going the other way.
//!
//! The acceleration is the north-east-down component, so down is positive.
//! More collective force accelerates the vehicle upward, which is a negative
//! down acceleration; the directional response below carries that inversion
//! once, so every metric reads positive when the vehicle obeyed.

use serde::{Deserialize, Serialize};

use crate::series::{
    first_crossing, timed_window, validate_event, validate_metric_results, validate_timed_values,
    validate_values,
};
use crate::{MetricError, TimedValue};

/// The fraction of the peak response that marks a detected answer.
pub const COLLECTIVE_DELAY_FRACTION: f64 = 0.02;
/// The final interval used for the steady collective response.
pub const COLLECTIVE_STEADY_WINDOW_S: f64 = 0.50;
/// The smallest commanded force change that carries a measurable response.
pub const MINIMUM_COLLECTIVE_DELTA: f64 = 1.0e-6;

/// The input event and the run-resolved forces for one collective step.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CollectiveStepSpec {
    /// The input event time, in seconds.
    pub input_time_s: f64,
    /// The effective collective force before the step, normalized against the
    /// identified hover force.
    pub baseline_force: f64,
    /// The effective collective force the stimulus requested, normalized
    /// against the identified hover force.
    pub target_force: f64,
}

/// Continuous metrics for one collective force response.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CollectiveMetrics {
    /// The commanded change in normalized collective force.
    pub commanded_force_delta: f64,
    /// Time from input to 2 percent of the peak answer, in seconds.
    pub input_to_response_delay_s: Option<f64>,
    /// The largest acceleration in the commanded direction, in m/s squared.
    pub peak_response_mps2: f64,
    /// The mean acceleration in the commanded direction over the final
    /// window, in m/s squared.
    pub steady_response_mps2: f64,
    /// The fraction of the response window spent accelerating against the
    /// commanded direction.
    pub direction_error_fraction: f64,
}

/// Calculates collective response metrics from a saved acceleration series.
///
/// # Errors
///
/// Returns [`MetricError`] when the series is invalid, when the input event is
/// outside the series, or when the commanded force change is too small to
/// measure.
pub fn measure_collective_response(
    down_acceleration_mps2: &[TimedValue],
    spec: CollectiveStepSpec,
) -> Result<CollectiveMetrics, MetricError> {
    let commanded_force_delta = validate_inputs(down_acceleration_mps2, spec)?;
    let response = directional_response(down_acceleration_mps2, spec, commanded_force_delta);
    validate_values(&response, "collective.response", |sample| sample.value)?;
    let peak_response_mps2 = response
        .iter()
        .map(|sample| sample.value)
        .fold(f64::NEG_INFINITY, f64::max);
    let metrics = CollectiveMetrics {
        commanded_force_delta,
        input_to_response_delay_s: detection_delay(&response, peak_response_mps2, spec),
        peak_response_mps2,
        steady_response_mps2: steady_response(&response),
        direction_error_fraction: direction_error_fraction(&response),
    };
    validate_metric_results(&[
        (
            "collective.commanded_force_delta",
            metrics.commanded_force_delta,
        ),
        ("collective.peak_response_mps2", metrics.peak_response_mps2),
        (
            "collective.steady_response_mps2",
            metrics.steady_response_mps2,
        ),
        (
            "collective.direction_error_fraction",
            metrics.direction_error_fraction,
        ),
    ])?;
    Ok(metrics)
}

fn validate_inputs(
    down_acceleration_mps2: &[TimedValue],
    spec: CollectiveStepSpec,
) -> Result<f64, MetricError> {
    validate_timed_values(down_acceleration_mps2)?;
    for (field, value) in [
        ("input_time_s", spec.input_time_s),
        ("baseline_force", spec.baseline_force),
        ("target_force", spec.target_force),
    ] {
        if !value.is_finite() {
            return Err(MetricError::InvalidParameter { field });
        }
    }
    let delta = spec.target_force - spec.baseline_force;
    if delta.abs() < MINIMUM_COLLECTIVE_DELTA {
        return Err(MetricError::ZeroStep);
    }
    validate_event(
        down_acceleration_mps2,
        "input",
        spec.input_time_s,
        |sample| sample.time_s,
    )?;
    if spec.input_time_s >= down_acceleration_mps2[down_acceleration_mps2.len() - 1].time_s {
        return Err(MetricError::InvalidParameter {
            field: "input_time_s",
        });
    }
    Ok(delta)
}

/// The acceleration resolved along the commanded force direction.
///
/// More collective force is an upward acceleration, so the down component is
/// negated once here. A value above zero is the vehicle obeying, whichever way
/// the collective was moved.
fn directional_response(
    down_acceleration_mps2: &[TimedValue],
    spec: CollectiveStepSpec,
    commanded_force_delta: f64,
) -> Vec<TimedValue> {
    let end_s = down_acceleration_mps2[down_acceleration_mps2.len() - 1].time_s;
    let direction = commanded_force_delta.signum();
    timed_window(down_acceleration_mps2, spec.input_time_s, end_s)
        .into_iter()
        .map(|sample| TimedValue {
            time_s: sample.time_s,
            value: -sample.value * direction,
        })
        .collect()
}

fn detection_delay(
    response: &[TimedValue],
    peak_response_mps2: f64,
    spec: CollectiveStepSpec,
) -> Option<f64> {
    if peak_response_mps2 <= 0.0 {
        return None;
    }
    first_crossing(response, peak_response_mps2 * COLLECTIVE_DELAY_FRACTION)
        .map(|time_s| time_s - spec.input_time_s)
}

fn steady_response(response: &[TimedValue]) -> f64 {
    let end_s = response[response.len() - 1].time_s;
    let start_s = (end_s - COLLECTIVE_STEADY_WINDOW_S).max(response[0].time_s);
    if end_s <= start_s {
        return response[response.len() - 1].value;
    }
    let window = timed_window(response, start_s, end_s);
    let integral = window
        .windows(2)
        .map(|pair| (pair[0].value + pair[1].value) * (pair[1].time_s - pair[0].time_s) / 2.0)
        .sum::<f64>();
    integral / (end_s - start_s)
}

/// The share of the response window spent accelerating the wrong way.
fn direction_error_fraction(response: &[TimedValue]) -> f64 {
    let total_s = response[response.len() - 1].time_s - response[0].time_s;
    if total_s <= 0.0 {
        return 0.0;
    }
    let against_s = response
        .windows(2)
        .map(|pair| negative_duration(pair[0], pair[1]))
        .sum::<f64>();
    (against_s / total_s).clamp(0.0, 1.0)
}

fn negative_duration(start: TimedValue, end: TimedValue) -> f64 {
    let duration_s = end.time_s - start.time_s;
    if start.value < 0.0 && end.value < 0.0 {
        return duration_s;
    }
    if start.value >= 0.0 && end.value >= 0.0 {
        return 0.0;
    }
    let crossing = start.value / (start.value - end.value);
    if start.value < 0.0 {
        duration_s * crossing
    } else {
        duration_s * (1.0 - crossing)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests;
