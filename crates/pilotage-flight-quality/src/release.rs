use serde::{Deserialize, Serialize};

use crate::series::{
    linear_value_at, timed_window, validate_event, validate_metric_results,
    validate_optional_metric_results, validate_times, validate_values,
};
use crate::{MetricError, MotionPoint, TimedValue};

/// The speed threshold that marks a stopped vehicle.
pub const STOP_SPEED_MPS: f64 = 0.05;
/// The time that speed must stay below the stop threshold.
pub const STOP_DWELL_S: f64 = 0.20;
/// The position-error hysteresis for a final-hold zero crossing.
pub const HOLD_ZERO_HYSTERESIS_M: f64 = 0.01;

/// Metrics from input release to final-hold entry.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseMetrics {
    /// Time from release to the first confirmed stop entry, in seconds.
    pub release_to_stop_s: Option<f64>,
    /// Travel in the release direction before the confirmed stop, in meters.
    pub brake_distance_m: Option<f64>,
    /// Travel back toward the release point before final hold, in meters.
    pub return_toward_release_m: f64,
    /// The first opposite-direction velocity excursion, in meters per second.
    pub opposite_velocity_peak_mps: f64,
}

/// Metrics for motion around the final hold point.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HoldMetrics {
    /// The largest opposite-side excursion after the first target crossing.
    pub rebound_distance_m: f64,
    /// The number of target crossings outside the fixed hysteresis.
    pub zero_crossings: u32,
}

/// Calculates release and brake metrics on one selected axis.
///
/// The function confirms a stop only when speed stays at or below 0.05 m/s
/// for 0.20 seconds. It measures release return before `hold_start_s`.
///
/// # Errors
///
/// Returns [`MetricError`] when the series or event times are invalid, or
/// when the vehicle has no clear velocity direction at release.
pub fn measure_release(
    motion: &[MotionPoint],
    release_time_s: f64,
    hold_start_s: f64,
) -> Result<ReleaseMetrics, MetricError> {
    validate_motion(motion, release_time_s, hold_start_s)?;
    let release_velocity = value_at(motion, release_time_s, |point| point.velocity_mps);
    validate_metric_results(&[("release.velocity_mps", release_velocity)])?;
    if release_velocity.abs() <= STOP_SPEED_MPS {
        return Err(MetricError::NoReleaseDirection);
    }
    let direction = release_velocity.signum();
    let stop_time = confirmed_stop_time(motion, release_time_s, hold_start_s)?;
    let release_position = value_at(motion, release_time_s, |point| point.position_m);
    validate_metric_results(&[("release.position_m", release_position)])?;
    let brake_distance = stop_time.map(|time_s| {
        maximum_directional_displacement(
            motion,
            release_time_s,
            time_s,
            release_position,
            direction,
        )
    });
    let metrics = ReleaseMetrics {
        release_to_stop_s: stop_time.map(|time_s| time_s - release_time_s),
        brake_distance_m: brake_distance,
        return_toward_release_m: release_return(
            motion,
            release_time_s,
            hold_start_s,
            release_position,
            direction,
        )?,
        opposite_velocity_peak_mps: opposite_velocity_peak(
            motion,
            release_time_s,
            hold_start_s,
            direction,
        ),
    };
    validate_optional_metric_results(&[
        ("release.release_to_stop_s", metrics.release_to_stop_s),
        ("release.brake_distance_m", metrics.brake_distance_m),
    ])?;
    validate_metric_results(&[
        (
            "release.return_toward_release_m",
            metrics.return_toward_release_m,
        ),
        (
            "release.opposite_velocity_peak_mps",
            metrics.opposite_velocity_peak_mps,
        ),
    ])?;
    Ok(metrics)
}

/// Calculates rebound and zero crossings around one final hold point.
///
/// # Errors
///
/// Returns [`MetricError`] when the position series or hold time is invalid.
pub fn measure_hold(
    position: &[TimedValue],
    hold_start_s: f64,
    hold_position_m: f64,
) -> Result<HoldMetrics, MetricError> {
    crate::series::validate_timed_values(position)?;
    validate_event(position, "hold_start", hold_start_s, |sample| sample.time_s)?;
    if !hold_position_m.is_finite() {
        return Err(MetricError::InvalidParameter {
            field: "hold_position_m",
        });
    }
    let end_s = position[position.len() - 1].time_s;
    if hold_start_s >= end_s {
        return Err(MetricError::InvalidParameter {
            field: "hold_start_s",
        });
    }
    let window = timed_window(position, hold_start_s, end_s);
    for sample in &window {
        validate_metric_results(&[("hold.position_error_m", sample.value - hold_position_m)])?;
    }
    let metrics = hold_excursions(&window, hold_position_m);
    validate_metric_results(&[("hold.rebound_distance_m", metrics.rebound_distance_m)])?;
    Ok(metrics)
}

fn validate_motion(
    motion: &[MotionPoint],
    release_time_s: f64,
    hold_start_s: f64,
) -> Result<(), MetricError> {
    validate_times(motion, |point| point.time_s)?;
    validate_values(motion, "position_m", |point| point.position_m)?;
    validate_values(motion, "velocity_mps", |point| point.velocity_mps)?;
    validate_event(motion, "release", release_time_s, |point| point.time_s)?;
    validate_event(motion, "hold_start", hold_start_s, |point| point.time_s)?;
    if hold_start_s <= release_time_s {
        return Err(MetricError::InvalidParameter {
            field: "hold_start_s",
        });
    }
    Ok(())
}

fn value_at(motion: &[MotionPoint], time_s: f64, value: impl Fn(&MotionPoint) -> f64) -> f64 {
    linear_value_at(motion, time_s, |point| point.time_s, value)
}

fn confirmed_stop_time(
    motion: &[MotionPoint],
    release_time_s: f64,
    hold_start_s: f64,
) -> Result<Option<f64>, MetricError> {
    let mut low_intervals = Vec::new();
    for pair in motion.windows(2) {
        let start = pair[0].time_s.max(release_time_s);
        let end = pair[1].time_s.min(hold_start_s);
        if start >= end {
            continue;
        }
        let v0 = value_at(motion, start, |point| point.velocity_mps);
        let v1 = value_at(motion, end, |point| point.velocity_mps);
        validate_metric_results(&[("release.stop_velocity_delta_mps", v1 - v0)])?;
        if let Some(interval) = low_speed_interval(start, v0, end, v1) {
            low_intervals.push(interval);
        }
    }
    Ok(first_dwell_interval(&low_intervals, STOP_DWELL_S))
}

fn low_speed_interval(t0: f64, v0: f64, t1: f64, v1: f64) -> Option<(f64, f64)> {
    if v0 == v1 {
        return (v0.abs() <= STOP_SPEED_MPS).then_some((t0, t1));
    }
    let delta = v1 - v0;
    let at_negative = (-STOP_SPEED_MPS - v0) / delta;
    let at_positive = (STOP_SPEED_MPS - v0) / delta;
    let start_fraction = at_negative.min(at_positive).clamp(0.0, 1.0);
    let end_fraction = at_negative.max(at_positive).clamp(0.0, 1.0);
    let middle = (start_fraction + end_fraction) / 2.0;
    let middle_velocity = v0 + middle * delta;
    if start_fraction > end_fraction || middle_velocity.abs() > STOP_SPEED_MPS {
        return None;
    }
    let duration = t1 - t0;
    Some((t0 + start_fraction * duration, t0 + end_fraction * duration))
}

fn first_dwell_interval(intervals: &[(f64, f64)], dwell_s: f64) -> Option<f64> {
    let mut merged: Option<(f64, f64)> = None;
    for &(start, end) in intervals {
        match merged {
            Some((first, prior_end)) if start <= prior_end + f64::EPSILON => {
                merged = Some((first, prior_end.max(end)));
            }
            Some((first, prior_end)) => {
                if prior_end - first >= dwell_s {
                    return Some(first);
                }
                merged = Some((start, end));
            }
            None => merged = Some((start, end)),
        }
    }
    merged.and_then(|(start, end)| (end - start >= dwell_s).then_some(start))
}

fn release_return(
    motion: &[MotionPoint],
    release_time_s: f64,
    hold_start_s: f64,
    release_position: f64,
    direction: f64,
) -> Result<f64, MetricError> {
    let mut running_maximum = 0.0_f64;
    let mut maximum_return = 0.0_f64;
    for position in values_in_window(motion, release_time_s, hold_start_s, |point| {
        point.position_m
    }) {
        let displacement = (position - release_position) * direction;
        validate_metric_results(&[("release.displacement_m", displacement)])?;
        running_maximum = running_maximum.max(displacement);
        let return_distance = running_maximum - displacement;
        validate_metric_results(&[("release.return_distance_m", return_distance)])?;
        maximum_return = maximum_return.max(return_distance);
    }
    Ok(maximum_return)
}

fn opposite_velocity_peak(
    motion: &[MotionPoint],
    release_time_s: f64,
    hold_start_s: f64,
    direction: f64,
) -> f64 {
    let mut active = false;
    let mut peak = 0.0_f64;
    for velocity in values_in_window(motion, release_time_s, hold_start_s, |point| {
        point.velocity_mps
    }) {
        let directed = velocity * direction;
        if !active {
            if directed < -STOP_SPEED_MPS {
                active = true;
                peak = -directed;
            }
        } else if directed >= -STOP_SPEED_MPS {
            break;
        } else {
            peak = peak.max(-directed);
        }
    }
    peak
}

fn maximum_directional_displacement(
    motion: &[MotionPoint],
    start_s: f64,
    end_s: f64,
    origin_m: f64,
    direction: f64,
) -> f64 {
    values_in_window(motion, start_s, end_s, |point| point.position_m)
        .into_iter()
        .map(|position| (position - origin_m) * direction)
        .fold(0.0_f64, f64::max)
}

fn values_in_window(
    motion: &[MotionPoint],
    start_s: f64,
    end_s: f64,
    value: impl Fn(&MotionPoint) -> f64 + Copy,
) -> Vec<f64> {
    let mut values = Vec::with_capacity(motion.len() + 2);
    values.push(value_at(motion, start_s, value));
    values.extend(
        motion
            .iter()
            .filter(|point| point.time_s > start_s && point.time_s < end_s)
            .map(value),
    );
    values.push(value_at(motion, end_s, value));
    values
}

fn hold_excursions(window: &[TimedValue], target: f64) -> HoldMetrics {
    let mut first_sign: Option<i8> = None;
    let mut current_sign: Option<i8> = None;
    let mut crossings = 0_u32;
    let mut rebound = 0.0_f64;
    for sample in window {
        let error = sample.value - target;
        let sign = hysteresis_sign(error);
        let Some(sign) = sign else {
            continue;
        };
        first_sign.get_or_insert(sign);
        if current_sign.is_some_and(|prior| prior != sign) {
            crossings = crossings.wrapping_add(1);
        }
        current_sign = Some(sign);
        if crossings > 0 && first_sign.is_some_and(|initial| initial != sign) {
            rebound = rebound.max(error.abs());
        }
    }
    HoldMetrics {
        rebound_distance_m: rebound,
        zero_crossings: crossings,
    }
}

fn hysteresis_sign(error: f64) -> Option<i8> {
    if error > HOLD_ZERO_HYSTERESIS_M {
        Some(1)
    } else if error < -HOLD_ZERO_HYSTERESIS_M {
        Some(-1)
    } else {
        None
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests;
