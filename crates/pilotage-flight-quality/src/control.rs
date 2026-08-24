use serde::{Deserialize, Serialize};

use crate::series::{
    integral_abs_linear, integral_square_linear, validate_metric_results, validate_times,
    validate_values,
};
use crate::{ControlPoint, MetricError};

/// Control-effort and actuator-saturation metrics.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlMetrics {
    /// The root mean square normalized control effort.
    pub effort_rms: f64,
    /// The time integral of absolute normalized control effort, in seconds.
    pub integrated_abs_effort_s: f64,
    /// The fraction of phase time with active saturation.
    pub saturation_fraction: f64,
    /// The longest continuous saturation interval, in seconds.
    pub longest_saturation_s: f64,
}

/// Calculates control effort and saturation metrics.
///
/// Effort is linear between samples. Saturation is constant from one sample
/// until the next sample. The last sample has no independent duration.
///
/// # Errors
///
/// Returns [`MetricError`] when the series is invalid.
pub fn measure_control(samples: &[ControlPoint]) -> Result<ControlMetrics, MetricError> {
    validate_times(samples, |sample| sample.time_s)?;
    validate_values(samples, "effort", |sample| sample.effort)?;
    let duration_s = samples[samples.len() - 1].time_s - samples[0].time_s;
    let mut square_integral = 0.0;
    let mut absolute_integral = 0.0;
    let mut saturated_s = 0.0;
    let mut longest_saturation_s = 0.0_f64;
    let mut active_saturation_s = 0.0;
    for pair in samples.windows(2) {
        let dt = pair[1].time_s - pair[0].time_s;
        square_integral += integral_square_linear(pair[0].effort, pair[1].effort, dt);
        absolute_integral += integral_abs_linear(pair[0].effort, pair[1].effort, dt);
        if pair[0].saturated {
            saturated_s += dt;
            active_saturation_s += dt;
            longest_saturation_s = longest_saturation_s.max(active_saturation_s);
        } else {
            active_saturation_s = 0.0;
        }
    }
    let metrics = ControlMetrics {
        effort_rms: (square_integral / duration_s).sqrt(),
        integrated_abs_effort_s: absolute_integral,
        saturation_fraction: saturated_s / duration_s,
        longest_saturation_s,
    };
    validate_metric_results(&[
        ("control.effort_rms", metrics.effort_rms),
        (
            "control.integrated_abs_effort_s",
            metrics.integrated_abs_effort_s,
        ),
        ("control.saturation_fraction", metrics.saturation_fraction),
        ("control.longest_saturation_s", metrics.longest_saturation_s),
    ])?;
    Ok(metrics)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests;
