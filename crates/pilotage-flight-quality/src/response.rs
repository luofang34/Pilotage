use serde::{Deserialize, Serialize};

use crate::series::{
    band_settling_time, crossing_delta, first_crossing, integral_abs_linear, timed_window,
    validate_event, validate_metric_results, validate_optional_metric_results,
    validate_timed_values, validate_values,
};
use crate::{MetricError, TimedValue};

/// The directional step fraction that marks a detected response.
pub const DELAY_FRACTION: f64 = 0.02;
/// The lower directional fraction for rise time.
pub const RISE_LOW_FRACTION: f64 = 0.10;
/// The upper directional fraction for rise time.
pub const RISE_HIGH_FRACTION: f64 = 0.90;
/// The half-width of the settling band around the target.
pub const SETTLING_FRACTION: f64 = 0.05;
/// The final response interval used for steady-state error.
pub const STEADY_STATE_WINDOW_S: f64 = 0.50;

/// The input event and values for one scalar step.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepSpec {
    /// The input event time, in seconds.
    pub input_time_s: f64,
    /// The value before the step.
    pub initial_value: f64,
    /// The requested final value.
    pub target_value: f64,
}

/// Continuous metrics for one scalar step response.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseMetrics {
    /// Time from input to 2 percent of the command step, in seconds.
    pub input_to_command_delay_s: Option<f64>,
    /// Time from input to 2 percent of the response step, in seconds.
    pub input_to_response_delay_s: Option<f64>,
    /// Time from 10 percent to 90 percent of the response, in seconds.
    pub rise_time_s: Option<f64>,
    /// Time from input to final entry in the 5 percent band, in seconds.
    pub settling_time_s: Option<f64>,
    /// The largest response beyond the target, in response units.
    pub overshoot: f64,
    /// Overshoot divided by the absolute step amplitude.
    pub overshoot_fraction: f64,
    /// The largest response behind the initial value, in response units.
    pub undershoot: f64,
    /// The signed mean response error in the final fixed window.
    pub steady_state_error: f64,
    /// The time integral of absolute response error after the input event.
    pub integrated_absolute_error: f64,
}

/// Calculates fixed step-response metrics from saved scalar series.
///
/// The function uses the first directional threshold crossing. It uses the
/// last entry into the settling band. It returns `None` when a threshold is
/// not reached before the series ends.
///
/// # Errors
///
/// Returns [`MetricError`] when a series is invalid, an event is outside a
/// series, or the step amplitude is zero.
pub fn measure_step_response(
    command: &[TimedValue],
    response: &[TimedValue],
    spec: StepSpec,
) -> Result<ResponseMetrics, MetricError> {
    validate_inputs(command, response, spec)?;
    let command_progress = progress_window(command, spec);
    validate_values(&command_progress, "response.command_progress", |sample| {
        sample.value
    })?;
    let response_progress = progress_window(response, spec);
    validate_values(&response_progress, "response.response_progress", |sample| {
        sample.value
    })?;
    let command_delay =
        first_crossing(&command_progress, DELAY_FRACTION).map(|time_s| time_s - spec.input_time_s);
    let response_delay =
        first_crossing(&response_progress, DELAY_FRACTION).map(|time_s| time_s - spec.input_time_s);
    let rise_time = crossing_delta(&response_progress, RISE_LOW_FRACTION, RISE_HIGH_FRACTION);
    let settling_time = band_settling_time(&response_progress, SETTLING_FRACTION)
        .map(|time_s| time_s - spec.input_time_s);
    let (overshoot, undershoot) = excursions(&response_progress, spec);
    let response_error = response_error(response, spec);
    validate_values(&response_error, "response.error", |sample| sample.value)?;
    let amplitude = (spec.target_value - spec.initial_value).abs();
    let metrics = ResponseMetrics {
        input_to_command_delay_s: command_delay,
        input_to_response_delay_s: response_delay,
        rise_time_s: rise_time,
        settling_time_s: settling_time,
        overshoot,
        overshoot_fraction: overshoot / amplitude,
        undershoot,
        steady_state_error: steady_state_error(&response_error),
        integrated_absolute_error: integrated_absolute_error(&response_error),
    };
    validate_response_metrics(metrics)
}

fn validate_response_metrics(metrics: ResponseMetrics) -> Result<ResponseMetrics, MetricError> {
    validate_optional_metric_results(&[
        (
            "response.input_to_command_delay_s",
            metrics.input_to_command_delay_s,
        ),
        (
            "response.input_to_response_delay_s",
            metrics.input_to_response_delay_s,
        ),
        ("response.rise_time_s", metrics.rise_time_s),
        ("response.settling_time_s", metrics.settling_time_s),
    ])?;
    validate_metric_results(&[
        ("response.overshoot", metrics.overshoot),
        ("response.overshoot_fraction", metrics.overshoot_fraction),
        ("response.undershoot", metrics.undershoot),
        ("response.steady_state_error", metrics.steady_state_error),
        (
            "response.integrated_absolute_error",
            metrics.integrated_absolute_error,
        ),
    ])?;
    Ok(metrics)
}

fn validate_inputs(
    command: &[TimedValue],
    response: &[TimedValue],
    spec: StepSpec,
) -> Result<(), MetricError> {
    validate_timed_values(command)?;
    validate_timed_values(response)?;
    for (field, value) in [
        ("input_time_s", spec.input_time_s),
        ("initial_value", spec.initial_value),
        ("target_value", spec.target_value),
    ] {
        if !value.is_finite() {
            return Err(MetricError::InvalidParameter { field });
        }
    }
    if spec.initial_value == spec.target_value {
        return Err(MetricError::ZeroStep);
    }
    validate_metric_results(&[(
        "response.step_delta",
        spec.target_value - spec.initial_value,
    )])?;
    validate_event(command, "input", spec.input_time_s, |sample| sample.time_s)?;
    validate_event(response, "input", spec.input_time_s, |sample| sample.time_s)?;
    if spec.input_time_s >= command[command.len() - 1].time_s
        || spec.input_time_s >= response[response.len() - 1].time_s
    {
        return Err(MetricError::InvalidParameter {
            field: "input_time_s",
        });
    }
    Ok(())
}

fn progress_window(samples: &[TimedValue], spec: StepSpec) -> Vec<TimedValue> {
    let end_s = samples[samples.len() - 1].time_s;
    let amplitude = (spec.target_value - spec.initial_value).abs();
    let direction = (spec.target_value - spec.initial_value).signum();
    timed_window(samples, spec.input_time_s, end_s)
        .into_iter()
        .map(|sample| TimedValue {
            time_s: sample.time_s,
            value: (sample.value - spec.initial_value) * direction / amplitude,
        })
        .collect()
}

fn excursions(progress: &[TimedValue], spec: StepSpec) -> (f64, f64) {
    let maximum = progress
        .iter()
        .map(|sample| sample.value)
        .fold(f64::NEG_INFINITY, f64::max);
    let minimum = progress
        .iter()
        .map(|sample| sample.value)
        .fold(f64::INFINITY, f64::min);
    let amplitude = (spec.target_value - spec.initial_value).abs();
    (
        (maximum - 1.0).max(0.0) * amplitude,
        (-minimum).max(0.0) * amplitude,
    )
}

fn response_error(samples: &[TimedValue], spec: StepSpec) -> Vec<TimedValue> {
    let end_s = samples[samples.len() - 1].time_s;
    timed_window(samples, spec.input_time_s, end_s)
        .into_iter()
        .map(|sample| TimedValue {
            time_s: sample.time_s,
            value: sample.value - spec.target_value,
        })
        .collect()
}

fn steady_state_error(error: &[TimedValue]) -> f64 {
    let end_s = error[error.len() - 1].time_s;
    let start_s = (end_s - STEADY_STATE_WINDOW_S).max(error[0].time_s);
    let window = timed_window(error, start_s, end_s);
    let integral = window
        .windows(2)
        .map(|pair| (pair[0].value + pair[1].value) * (pair[1].time_s - pair[0].time_s) / 2.0)
        .sum::<f64>();
    integral / (end_s - start_s)
}

fn integrated_absolute_error(error: &[TimedValue]) -> f64 {
    error
        .windows(2)
        .map(|pair| {
            integral_abs_linear(
                pair[0].value,
                pair[1].value,
                pair[1].time_s - pair[0].time_s,
            )
        })
        .sum()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests;
