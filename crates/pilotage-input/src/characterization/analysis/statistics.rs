//! Robust center and bounded timing statistics.

use crate::{
    CaptureSample, CenterBehavior, CharacterizationCapture, TimestampSource,
    TimingCharacterization, TimingObservation,
};

use super::AnalysisError;

#[derive(Debug, Clone, Copy)]
pub(super) struct CenterStats {
    pub(super) center: f32,
    pub(super) noise: f32,
    pub(super) drift_per_second: f32,
    pub(super) behavior: CenterBehavior,
}

pub(super) fn center_statistics(
    idle: &[&CaptureSample],
    timestamp_source: TimestampSource,
) -> Result<Vec<CenterStats>, AnalysisError> {
    let axis_count = idle
        .first()
        .map(|sample| sample.axes.len())
        .ok_or_else(|| invalid_error("the idle segment is empty"))?;
    (0..axis_count)
        .map(|axis| center_stat(idle, axis, timestamp_source))
        .collect()
}

fn center_stat(
    idle: &[&CaptureSample],
    axis: usize,
    timestamp_source: TimestampSource,
) -> Result<CenterStats, AnalysisError> {
    let times: Vec<f64> = idle
        .iter()
        .map(|sample| sample_time(sample, timestamp_source) as f64)
        .collect();
    let values: Vec<f64> = idle
        .iter()
        .map(|sample| f64::from(sample.axes[axis]))
        .collect();
    let center = median(&values)?;
    let (intercept, slope) = linear_fit(&times, &values)?;
    let residuals: Vec<f64> = times
        .iter()
        .zip(&values)
        .map(|(time, value)| (value - (intercept + slope * time)).abs())
        .collect();
    let noise = median(&residuals)?;
    let duration_us = times.last().copied().unwrap_or(0.0) - times.first().copied().unwrap_or(0.0);
    let total_drift = slope.abs() * duration_us;
    let behavior = if total_drift > (4.0 * noise).max(0.001) {
        CenterBehavior::Drifting
    } else {
        CenterBehavior::Stable
    };
    Ok(CenterStats {
        center: center as f32,
        noise: noise as f32,
        drift_per_second: (slope.abs() * 1_000_000.0) as f32,
        behavior,
    })
}

fn linear_fit(times: &[f64], values: &[f64]) -> Result<(f64, f64), AnalysisError> {
    if times.len() != values.len() || times.len() < 2 {
        return invalid("center fit needs two aligned samples");
    }
    let time_mean = times.iter().sum::<f64>() / times.len() as f64;
    let value_mean = values.iter().sum::<f64>() / values.len() as f64;
    let numerator: f64 = times
        .iter()
        .zip(values)
        .map(|(time, value)| (time - time_mean) * (value - value_mean))
        .sum();
    let denominator: f64 = times.iter().map(|time| (time - time_mean).powi(2)).sum();
    if denominator <= 0.0 {
        return invalid("center fit timestamps have no span");
    }
    let slope = numerator / denominator;
    Ok((value_mean - slope * time_mean, slope))
}

pub(super) fn characterize_timing(
    capture: &CharacterizationCapture,
) -> Result<TimingCharacterization, AnalysisError> {
    let deltas = segment_deltas(capture);
    let period = median(&deltas)?;
    if period <= 0.0 {
        return invalid("timing period is not positive");
    }
    let deviations: Vec<f64> = deltas.iter().map(|delta| (delta - period).abs()).collect();
    let jitter = median(&deviations)?;
    let dropped = match capture.timing_observation {
        TimingObservation::ReportCallbacks => Some(dropped_reports(&deltas, period)),
        TimingObservation::PolledStateUpdates | TimingObservation::InjectedSamples => None,
    };
    let sample_count = count(capture.samples.len());
    let loss_ratio = dropped.unwrap_or(0) as f32 / sample_count.max(1) as f32;
    let count_confidence = (sample_count as f32 / 25.0).clamp(0.0, 1.0);
    Ok(TimingCharacterization {
        observation: capture.timing_observation,
        sample_count,
        median_period_us: period,
        jitter_mad_us: jitter,
        dropped_report_count: dropped,
        confidence: count_confidence * (1.0 - loss_ratio).clamp(0.0, 1.0),
    })
}

fn segment_deltas(capture: &CharacterizationCapture) -> Vec<f64> {
    let mut deltas = Vec::new();
    let mut segment_index = 0;
    for pair in capture.samples.windows(2) {
        while capture
            .segments
            .get(segment_index)
            .is_some_and(|segment| pair[0].sequence > segment.end_sequence)
        {
            segment_index = segment_index.saturating_add(1);
        }
        let same_segment = capture.segments.get(segment_index).is_some_and(|segment| {
            pair[0].sequence >= segment.start_sequence && pair[1].sequence <= segment.end_sequence
        });
        if same_segment {
            deltas.push(
                sample_time(&pair[1], capture.timestamp_source)
                    .saturating_sub(sample_time(&pair[0], capture.timestamp_source))
                    as f64,
            );
        }
    }
    deltas
}

fn dropped_reports(deltas: &[f64], period: f64) -> u64 {
    deltas.iter().fold(0, |total, delta| {
        let missing = ((*delta / period).round() as u64).saturating_sub(1);
        total.saturating_add(missing)
    })
}

fn median(values: &[f64]) -> Result<f64, AnalysisError> {
    if values.is_empty() {
        return invalid("a median input is empty");
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        Ok((sorted[middle - 1] + sorted[middle]) / 2.0)
    } else {
        Ok(sorted[middle])
    }
}

pub(super) fn sample_time(sample: &CaptureSample, source: TimestampSource) -> u64 {
    match source {
        TimestampSource::Source => sample.source_at_us.unwrap_or(sample.observed_at_us),
        TimestampSource::Arrival => sample.observed_at_us,
    }
}

fn count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn invalid<T>(detail: &str) -> Result<T, AnalysisError> {
    Err(invalid_error(detail))
}

fn invalid_error(detail: &str) -> AnalysisError {
    AnalysisError::InvalidCapture {
        detail: detail.to_owned(),
    }
}
