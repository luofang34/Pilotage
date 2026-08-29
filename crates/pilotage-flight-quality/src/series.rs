use crate::{MetricError, TimedValue};

pub(crate) fn validate_times<T>(
    samples: &[T],
    time: impl Fn(&T) -> f64,
) -> Result<(), MetricError> {
    if samples.len() < 2 {
        return Err(MetricError::TooFewSamples {
            needed: 2,
            actual: samples.len(),
        });
    }
    let mut previous = time(&samples[0]);
    if !previous.is_finite() {
        return Err(MetricError::NonFiniteTime { index: 0 });
    }
    for (index, sample) in samples.iter().enumerate().skip(1) {
        let current = time(sample);
        if !current.is_finite() {
            return Err(MetricError::NonFiniteTime { index });
        }
        if current <= previous {
            return Err(MetricError::NonMonotonicTime {
                index,
                previous_s: previous,
                current_s: current,
            });
        }
        previous = current;
    }
    Ok(())
}

pub(crate) fn validate_values<T>(
    samples: &[T],
    field: &'static str,
    value: impl Fn(&T) -> f64,
) -> Result<(), MetricError> {
    for (index, sample) in samples.iter().enumerate() {
        if !value(sample).is_finite() {
            return Err(MetricError::NonFiniteValue { index, field });
        }
    }
    Ok(())
}

pub(crate) fn validate_timed_values(samples: &[TimedValue]) -> Result<(), MetricError> {
    validate_times(samples, |sample| sample.time_s)?;
    validate_values(samples, "value", |sample| sample.value)
}

pub(crate) fn validate_metric_results(results: &[(&'static str, f64)]) -> Result<(), MetricError> {
    for &(field, value) in results {
        if !value.is_finite() {
            return Err(MetricError::NonFiniteResult { field });
        }
    }
    Ok(())
}

pub(crate) fn validate_optional_metric_results(
    results: &[(&'static str, Option<f64>)],
) -> Result<(), MetricError> {
    for &(field, value) in results {
        if value.is_some_and(|value| !value.is_finite()) {
            return Err(MetricError::NonFiniteResult { field });
        }
    }
    Ok(())
}

pub(crate) fn validate_event<T>(
    samples: &[T],
    field: &'static str,
    time_s: f64,
    time: impl Fn(&T) -> f64,
) -> Result<(), MetricError> {
    if !time_s.is_finite()
        || time_s < time(&samples[0])
        || time_s > time(&samples[samples.len() - 1])
    {
        return Err(MetricError::EventOutsideSeries { field, time_s });
    }
    Ok(())
}

pub(crate) fn linear_value_at<T>(
    samples: &[T],
    at_s: f64,
    time: impl Fn(&T) -> f64,
    value: impl Fn(&T) -> f64,
) -> f64 {
    match samples.binary_search_by(|sample| {
        let sample_time = time(sample);
        if sample_time == at_s {
            std::cmp::Ordering::Equal
        } else {
            sample_time.total_cmp(&at_s)
        }
    }) {
        Ok(index) => value(&samples[index]),
        Err(index) => {
            let before = &samples[index - 1];
            let after = &samples[index];
            interpolate(time(before), value(before), time(after), value(after), at_s)
        }
    }
}

pub(crate) fn interpolate(t0: f64, v0: f64, t1: f64, v1: f64, at_s: f64) -> f64 {
    let fraction = (at_s - t0) / (t1 - t0);
    v0 + fraction * (v1 - v0)
}

pub(crate) fn timed_window(samples: &[TimedValue], start_s: f64, end_s: f64) -> Vec<TimedValue> {
    let mut window = Vec::with_capacity(samples.len() + 2);
    window.push(TimedValue {
        time_s: start_s,
        value: linear_value_at(
            samples,
            start_s,
            |sample| sample.time_s,
            |sample| sample.value,
        ),
    });
    window.extend(
        samples
            .iter()
            .copied()
            .filter(|sample| sample.time_s > start_s && sample.time_s < end_s),
    );
    if end_s > start_s {
        window.push(TimedValue {
            time_s: end_s,
            value: linear_value_at(
                samples,
                end_s,
                |sample| sample.time_s,
                |sample| sample.value,
            ),
        });
    }
    window
}

/// The first time a normalized progress series reaches a threshold.
///
/// The series starts at the input event, so a series that already sits at or
/// above the threshold crossed it at the event itself.
pub(crate) fn first_crossing(progress: &[TimedValue], threshold: f64) -> Option<f64> {
    if progress[0].value >= threshold {
        return Some(progress[0].time_s);
    }
    progress.windows(2).find_map(|pair| {
        let before = pair[0];
        let after = pair[1];
        if before.value < threshold && after.value >= threshold {
            Some(crossing_time(before, after, threshold))
        } else {
            None
        }
    })
}

/// The time at which a linear segment takes one value.
///
/// The interpolation runs from value to time, which is the inverse of the
/// usual direction: the question is when a threshold was reached, not what the
/// series held at a time.
pub(crate) fn crossing_time(before: TimedValue, after: TimedValue, value: f64) -> f64 {
    interpolate(
        before.value,
        before.time_s,
        after.value,
        after.time_s,
        value,
    )
}

/// The interval between two rising threshold crossings.
pub(crate) fn crossing_delta(progress: &[TimedValue], low: f64, high: f64) -> Option<f64> {
    let low_time = first_crossing(progress, low)?;
    let high_time = first_crossing(progress, high)?;
    Some(high_time - low_time)
}

/// The time of the final entry into a band around unit progress.
///
/// A series that ends outside the band never settled, which is a measurement
/// the series does not carry rather than a value to guess.
pub(crate) fn band_settling_time(progress: &[TimedValue], half_width: f64) -> Option<f64> {
    let low = 1.0 - half_width;
    let high = 1.0 + half_width;
    if !(low..=high).contains(&progress[progress.len() - 1].value) {
        return None;
    }
    let last_outside = progress
        .iter()
        .rposition(|sample| !(low..=high).contains(&sample.value));
    let Some(index) = last_outside else {
        return Some(progress[0].time_s);
    };
    let before = progress[index];
    let after = progress[index + 1];
    let boundary = if before.value < low { low } else { high };
    Some(crossing_time(before, after, boundary))
}

pub(crate) fn integral_square_linear(v0: f64, v1: f64, duration_s: f64) -> f64 {
    duration_s * (v0 * v0 + v0 * v1 + v1 * v1) / 3.0
}

pub(crate) fn integral_abs_linear(v0: f64, v1: f64, duration_s: f64) -> f64 {
    if v0 * v1 >= 0.0 {
        return duration_s * (v0.abs() + v1.abs()) / 2.0;
    }
    let total = v0.abs() + v1.abs();
    let first_fraction = v0.abs() / total;
    duration_s * (v0.abs() * first_fraction + v1.abs() * (1.0 - first_fraction)) / 2.0
}

pub(crate) fn weighted_percentile(values: &mut [(f64, f64)], quantile: f64) -> f64 {
    values.sort_by(|left, right| left.0.total_cmp(&right.0));
    let total_weight: f64 = values.iter().map(|entry| entry.1).sum();
    let threshold = total_weight * quantile;
    let mut cumulative = 0.0;
    for &(value, weight) in values.iter() {
        cumulative += weight;
        if cumulative >= threshold {
            return value;
        }
    }
    values.last().map_or(0.0, |entry| entry.0)
}

pub(crate) fn weighted_abs_percentile_linear(samples: &[TimedValue], quantile: f64) -> f64 {
    let duration_s = samples[samples.len() - 1].time_s - samples[0].time_s;
    let target_duration_s = duration_s * quantile;
    let upper = samples
        .iter()
        .map(|sample| sample.value.abs())
        .fold(0.0_f64, f64::max);
    let mut lower_bits = 0_u64;
    let mut upper_bits = upper.to_bits();
    while lower_bits < upper_bits {
        let candidate_bits = lower_bits + (upper_bits - lower_bits) / 2;
        let candidate = f64::from_bits(candidate_bits);
        if duration_at_or_below(samples, candidate) >= target_duration_s {
            upper_bits = candidate_bits;
        } else {
            lower_bits = candidate_bits + 1;
        }
    }
    f64::from_bits(lower_bits)
}

fn duration_at_or_below(samples: &[TimedValue], threshold: f64) -> f64 {
    samples
        .windows(2)
        .map(|pair| segment_duration_at_or_below(pair[0], pair[1], threshold))
        .sum()
}

fn segment_duration_at_or_below(start: TimedValue, end: TimedValue, threshold: f64) -> f64 {
    let duration_s = end.time_s - start.time_s;
    let delta = end.value - start.value;
    if delta == 0.0 {
        return if start.value.abs() <= threshold {
            duration_s
        } else {
            0.0
        };
    }
    let negative_crossing = (-threshold - start.value) / delta;
    let positive_crossing = (threshold - start.value) / delta;
    let first = negative_crossing.min(positive_crossing).clamp(0.0, 1.0);
    let last = negative_crossing.max(positive_crossing).clamp(0.0, 1.0);
    (last - first) * duration_s
}

#[cfg(test)]
mod tests;
