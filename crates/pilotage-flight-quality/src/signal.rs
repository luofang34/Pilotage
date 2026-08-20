use serde::{Deserialize, Serialize};

use crate::series::{
    integral_square_linear, validate_metric_results, validate_timed_values,
    weighted_abs_percentile_linear, weighted_percentile,
};
use crate::{MetricError, TimedValue};

/// Time-weighted statistics for one scalar signal.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignalStats {
    /// The root mean square value in the signal unit.
    pub rms: f64,
    /// The duration-weighted 95th percentile of absolute value.
    pub p95_abs: f64,
    /// The largest absolute value.
    pub peak_abs: f64,
}

/// Acceleration and jerk metrics for one selected axis.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JerkMetrics {
    /// The largest absolute acceleration, in meters per second squared.
    pub peak_acceleration_mps2: f64,
    /// The largest absolute jerk, in meters per second cubed.
    pub peak_jerk_mps3: f64,
    /// The duration-weighted 95th percentile of absolute jerk.
    pub jerk_p95_mps3: f64,
    /// The root mean square jerk, in meters per second cubed.
    pub jerk_rms_mps3: f64,
    /// The integral of absolute jerk, in meters per second squared.
    pub integrated_abs_jerk_mps2: f64,
}

/// Calculates RMS, P95 absolute value, and peak absolute value.
///
/// RMS uses the exact square integral for each linear sample segment. P95
/// uses the time distribution of the absolute piecewise-linear signal.
///
/// # Errors
///
/// Returns [`MetricError`] when the series is invalid.
pub fn measure_signal(samples: &[TimedValue]) -> Result<SignalStats, MetricError> {
    validate_timed_values(samples)?;
    let duration_s = samples[samples.len() - 1].time_s - samples[0].time_s;
    let mut square_integral = 0.0;
    let mut peak = 0.0_f64;
    for pair in samples.windows(2) {
        let dt = pair[1].time_s - pair[0].time_s;
        square_integral += integral_square_linear(pair[0].value, pair[1].value, dt);
        peak = peak.max(pair[0].value.abs()).max(pair[1].value.abs());
    }
    let metrics = SignalStats {
        rms: (square_integral / duration_s).sqrt(),
        p95_abs: weighted_abs_percentile_linear(samples, 0.95),
        peak_abs: peak,
    };
    validate_metric_results(&[
        ("signal.rms", metrics.rms),
        ("signal.p95_abs", metrics.p95_abs),
        ("signal.peak_abs", metrics.peak_abs),
    ])?;
    Ok(metrics)
}

/// Calculates acceleration and jerk metrics.
///
/// The function treats acceleration as linear between samples. Jerk is the
/// constant slope on each sample interval. The function does not filter data.
///
/// # Errors
///
/// Returns [`MetricError`] when the acceleration series is invalid.
pub fn measure_jerk(acceleration: &[TimedValue]) -> Result<JerkMetrics, MetricError> {
    validate_timed_values(acceleration)?;
    let duration_s = acceleration[acceleration.len() - 1].time_s - acceleration[0].time_s;
    let mut peak_acceleration = 0.0_f64;
    let mut peak_jerk = 0.0_f64;
    let mut jerk_square_integral = 0.0;
    let mut integrated_abs_jerk = 0.0;
    let mut weighted_jerk = Vec::with_capacity(acceleration.len() - 1);
    for pair in acceleration.windows(2) {
        let dt = pair[1].time_s - pair[0].time_s;
        let jerk = (pair[1].value - pair[0].value) / dt;
        peak_acceleration = peak_acceleration
            .max(pair[0].value.abs())
            .max(pair[1].value.abs());
        peak_jerk = peak_jerk.max(jerk.abs());
        jerk_square_integral += jerk * jerk * dt;
        integrated_abs_jerk += jerk.abs() * dt;
        weighted_jerk.push((jerk.abs(), dt));
    }
    let metrics = JerkMetrics {
        peak_acceleration_mps2: peak_acceleration,
        peak_jerk_mps3: peak_jerk,
        jerk_p95_mps3: weighted_percentile(&mut weighted_jerk, 0.95),
        jerk_rms_mps3: (jerk_square_integral / duration_s).sqrt(),
        integrated_abs_jerk_mps2: integrated_abs_jerk,
    };
    validate_metric_results(&[
        (
            "jerk.peak_acceleration_mps2",
            metrics.peak_acceleration_mps2,
        ),
        ("jerk.peak_jerk_mps3", metrics.peak_jerk_mps3),
        ("jerk.jerk_p95_mps3", metrics.jerk_p95_mps3),
        ("jerk.jerk_rms_mps3", metrics.jerk_rms_mps3),
        (
            "jerk.integrated_abs_jerk_mps2",
            metrics.integrated_abs_jerk_mps2,
        ),
    ])?;
    Ok(metrics)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests;
