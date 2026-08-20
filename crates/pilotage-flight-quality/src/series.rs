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
    match samples.binary_search_by(|sample| time(sample).total_cmp(&at_s)) {
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
