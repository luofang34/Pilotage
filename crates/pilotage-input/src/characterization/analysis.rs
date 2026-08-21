//! Deterministic characterization of one exact portable HID capture.

use std::collections::HashMap;

use crate::{
    AxisCalibration, AxisCharacterization, CALIBRATION_CANDIDATE_SCHEMA_VERSION,
    CalibrationCandidate, CaptureSample, CaptureSegment, CaptureSegmentKind,
    CharacterizationCapture, DeadzoneEvidenceStatus, DeviceProfile,
    MAX_CHARACTERIZATION_CAPTURE_BYTES, SourceAxisContract, SourceAxisRange, TimestampSource,
    TimingCharacterization, content_digest, parse_profile_bytes,
};

mod error;
mod statistics;
mod validation;

pub use error::AnalysisError;
use statistics::{CenterStats, center_statistics, characterize_timing, sample_time};
use validation::{validate_capture, validate_source_contract};

const MAX_CROSS_AXIS_COUPLING: f32 = 0.2;
const MIN_EXCURSION: f32 = 1.0e-4;

#[derive(Debug)]
struct MovementResult {
    axis: AxisCharacterization,
    selected_source: usize,
}

#[derive(Debug)]
struct AxisEvidence<'a> {
    logical: &'a str,
    source_index: usize,
    source_range: SourceAxisRange,
    movement: &'a [&'a CaptureSample],
    idle: &'a [&'a CaptureSample],
    center: CenterStats,
    coupling: f32,
    timing_confidence: f32,
    deadzone_status: DeadzoneEvidenceStatus,
    timestamp_source: TimestampSource,
}

/// Creates a calibration candidate from exact contract, capture, and profile bytes.
///
/// # Errors
///
/// Returns [`AnalysisError`] when an input cannot be parsed or its bounded
/// evidence is incomplete or inconsistent.
pub fn characterize_capture(
    source_contract_bytes: &[u8],
    capture_bytes: &[u8],
    baseline_profile_bytes: &[u8],
) -> Result<CalibrationCandidate, AnalysisError> {
    validate_capture_byte_count(capture_bytes.len())?;
    let contract: SourceAxisContract = serde_json::from_slice(source_contract_bytes)
        .map_err(|source| AnalysisError::ContractParse { source })?;
    let capture: CharacterizationCapture = serde_json::from_slice(capture_bytes)
        .map_err(|source| AnalysisError::CaptureParse { source })?;
    let profile = parse_profile_bytes(baseline_profile_bytes)
        .map_err(|source| AnalysisError::Profile { source })?;
    let contract_digest = digest_hex(source_contract_bytes);
    let raw_report_decoder =
        validate_source_contract(&contract, &contract_digest, &capture, &profile)?;
    validate_capture(&capture, &profile, raw_report_decoder.as_ref())?;
    characterize_validated(
        capture_bytes,
        &capture,
        baseline_profile_bytes,
        contract_digest,
        &profile,
    )
}

pub(super) fn validate_capture_byte_count(actual: usize) -> Result<(), AnalysisError> {
    if actual > MAX_CHARACTERIZATION_CAPTURE_BYTES {
        Err(AnalysisError::CaptureTooLarge {
            actual,
            limit: MAX_CHARACTERIZATION_CAPTURE_BYTES,
        })
    } else {
        Ok(())
    }
}

fn characterize_validated(
    capture_bytes: &[u8],
    capture: &CharacterizationCapture,
    baseline_profile_bytes: &[u8],
    source_contract_digest: String,
    profile: &DeviceProfile,
) -> Result<CalibrationCandidate, AnalysisError> {
    let idle = segment_samples(capture, idle_segment(capture)?)?;
    let center_stats = center_statistics(&idle, capture.timestamp_source)?;
    validate_idle_topology(&capture.source_axes, &center_stats)?;
    let timing = characterize_timing(capture)?;
    let axes = characterize_axes(capture, profile, &idle, &center_stats, &timing)?;
    let confidence = axes
        .iter()
        .map(|axis| axis.confidence)
        .chain(core::iter::once(timing.confidence))
        .fold(1.0f32, f32::min);
    Ok(CalibrationCandidate {
        schema_version: CALIBRATION_CANDIDATE_SCHEMA_VERSION,
        device: capture.device.clone(),
        source: capture.source,
        source_capture_digest: digest_hex(capture_bytes),
        baseline_profile_digest: digest_hex(baseline_profile_bytes),
        source_contract_digest,
        timing,
        deadzone_evidence: capture.deadzone_evidence.clone(),
        axes,
        sample_count: count(capture.samples.len()),
        confidence,
    })
}

fn validate_idle_topology(
    ranges: &[SourceAxisRange],
    centers: &[CenterStats],
) -> Result<(), AnalysisError> {
    const ENDPOINT_FRACTION: f32 = 0.05;
    for (range, center) in ranges.iter().zip(centers) {
        let span = range.maximum - range.minimum;
        let lower = center.center - range.minimum;
        let upper = range.maximum - center.center;
        let valid = match range.neutral_position {
            crate::NeutralPosition::Centered => {
                lower >= span * ENDPOINT_FRACTION && upper >= span * ENDPOINT_FRACTION
            }
            crate::NeutralPosition::Minimum => lower <= span * ENDPOINT_FRACTION,
            crate::NeutralPosition::Maximum => upper <= span * ENDPOINT_FRACTION,
        };
        if !valid {
            return invalid("an idle axis does not match its trusted neutral position");
        }
    }
    Ok(())
}

fn characterize_axes(
    capture: &CharacterizationCapture,
    profile: &DeviceProfile,
    idle: &[&CaptureSample],
    centers: &[CenterStats],
    timing: &TimingCharacterization,
) -> Result<Vec<AxisCharacterization>, AnalysisError> {
    let mut axes = Vec::new();
    let mut selected = HashMap::new();
    for segment in movement_segments(capture) {
        let result = characterize_movement(capture, profile, segment, idle, centers, timing)?;
        if let Some(first_logical) =
            selected.insert(result.selected_source, result.axis.logical.clone())
        {
            return Err(AnalysisError::DuplicateMovement {
                first_logical,
                second_logical: result.axis.logical,
                source_index: result.selected_source,
            });
        }
        axes.push(result.axis);
    }
    if axes.is_empty() {
        invalid("the capture has no movement segment")
    } else {
        Ok(axes)
    }
}

fn idle_segment(capture: &CharacterizationCapture) -> Result<&CaptureSegment, AnalysisError> {
    let mut idle = capture
        .segments
        .iter()
        .filter(|segment| matches!(segment.action, CaptureSegmentKind::Idle));
    let first = idle
        .next()
        .ok_or_else(|| invalid_error("the capture has no idle segment"))?;
    if idle.next().is_some() {
        return invalid("the capture has more than one idle segment");
    }
    Ok(first)
}

fn movement_segments(capture: &CharacterizationCapture) -> impl Iterator<Item = &CaptureSegment> {
    capture
        .segments
        .iter()
        .filter(|segment| matches!(segment.action, CaptureSegmentKind::Movement { .. }))
}

fn segment_samples<'a>(
    capture: &'a CharacterizationCapture,
    segment: &CaptureSegment,
) -> Result<Vec<&'a CaptureSample>, AnalysisError> {
    let samples: Vec<_> = capture
        .samples
        .iter()
        .filter(|sample| (segment.start_sequence..=segment.end_sequence).contains(&sample.sequence))
        .collect();
    if samples.len() < 4 {
        return invalid("each capture segment needs at least four samples");
    }
    Ok(samples)
}

fn characterize_movement(
    capture: &CharacterizationCapture,
    profile: &DeviceProfile,
    segment: &CaptureSegment,
    idle: &[&CaptureSample],
    centers: &[CenterStats],
    timing: &TimingCharacterization,
) -> Result<MovementResult, AnalysisError> {
    let (logical, positive_first) = movement_name(segment)?;
    if !positive_first {
        return invalid("a movement segment must move the positive direction first");
    }
    let samples = segment_samples(capture, segment)?;
    if !profile.axes.iter().any(|axis| axis.logical == logical) {
        return invalid("movement logical name is not in the baseline profile");
    }
    let peaks = peak_excursions(&samples, centers, &capture.source_axes);
    let selected = max_index(&peaks).ok_or_else(|| invalid_error("movement has no axes"))?;
    let primary = peaks[selected];
    let second = peaks
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != selected)
        .map(|(_, value)| *value)
        .fold(0.0f32, f32::max);
    let coupling = if primary > 0.0 { second / primary } else { 1.0 };
    let source_range = capture.source_axes[selected];
    let source_span = source_range.maximum - source_range.minimum;
    let threshold = (centers[selected].noise * 8.0 / source_span).max(MIN_EXCURSION);
    if primary < threshold || coupling > MAX_CROSS_AXIS_COUPLING {
        return Err(AnalysisError::AmbiguousMovement {
            logical: logical.to_owned(),
            source_index: selected,
            coupling,
        });
    }
    let axis = build_axis(AxisEvidence {
        logical,
        source_index: selected,
        source_range,
        movement: &samples,
        idle,
        center: centers[selected],
        coupling,
        timing_confidence: timing.confidence,
        deadzone_status: capture.deadzone_evidence.status,
        timestamp_source: capture.timestamp_source,
    })?;
    Ok(MovementResult {
        axis,
        selected_source: selected,
    })
}

fn build_axis(evidence: AxisEvidence<'_>) -> Result<AxisCharacterization, AnalysisError> {
    let values: Vec<f32> = evidence
        .movement
        .iter()
        .map(|sample| sample.axes[evidence.source_index])
        .collect();
    let observed_min = values
        .iter()
        .copied()
        .fold(evidence.center.center, f32::min);
    let observed_max = values
        .iter()
        .copied()
        .fold(evidence.center.center, f32::max);
    let span = (observed_max - observed_min).max(MIN_EXCURSION);
    let direction_threshold = (evidence.center.noise * 8.0).max(span * 0.05);
    let first_direction_excursion = values
        .iter()
        .map(|value| *value - evidence.center.center)
        .find(|excursion| excursion.abs() >= direction_threshold)
        .ok_or_else(|| invalid_error("movement has no directional excursion"))?;
    let idle_duration_us =
        evidence
            .idle
            .last()
            .zip(evidence.idle.first())
            .map_or(0, |(last, first)| {
                sample_time(last, evidence.timestamp_source)
                    .saturating_sub(sample_time(first, evidence.timestamp_source))
            });
    let mut axis = AxisCharacterization {
        logical: evidence.logical.to_owned(),
        source_index: evidence.source_index,
        source_range: evidence.source_range,
        invert: false,
        first_direction_excursion,
        calibration: AxisCalibration {
            min: observed_min,
            center: evidence.center.center,
            max: observed_max,
        },
        observed_min,
        observed_center: evidence.center.center,
        observed_max,
        center_noise: evidence.center.noise,
        center_drift_per_second: evidence.center.drift_per_second,
        idle_duration_us,
        center_behavior: evidence.center.behavior,
        cross_axis_coupling: evidence.coupling,
        proposed_deadzone: 0.0,
        idle_sample_count: count(evidence.idle.len()),
        movement_sample_count: count(evidence.movement.len()),
        confidence: 0.0,
    };
    if !axis.has_required_range() {
        return Err(AnalysisError::IncompleteMovement {
            logical: evidence.logical.to_owned(),
            source_index: evidence.source_index,
        });
    }
    axis.invert = axis.derived_invert();
    axis.calibration = axis.derived_calibration();
    axis.proposed_deadzone = axis.derived_deadzone(evidence.deadzone_status);
    axis.confidence = axis.derived_confidence(evidence.timing_confidence);
    Ok(axis)
}

fn movement_name(segment: &CaptureSegment) -> Result<(&str, bool), AnalysisError> {
    match &segment.action {
        CaptureSegmentKind::Movement {
            logical,
            positive_first,
        } if !logical.is_empty() => Ok((logical, *positive_first)),
        CaptureSegmentKind::Movement { .. } => invalid("movement logical name is empty"),
        CaptureSegmentKind::Idle => invalid("expected a movement segment"),
    }
}

fn peak_excursions(
    samples: &[&CaptureSample],
    centers: &[CenterStats],
    ranges: &[SourceAxisRange],
) -> Vec<f32> {
    centers
        .iter()
        .zip(ranges)
        .enumerate()
        .map(|(axis, (stats, range))| {
            let span = (range.maximum - range.minimum).max(MIN_EXCURSION);
            samples
                .iter()
                .map(|sample| (sample.axes[axis] - stats.center).abs() / span)
                .fold(0.0, f32::max)
        })
        .collect()
}

fn max_index(values: &[f32]) -> Option<usize> {
    values
        .iter()
        .enumerate()
        .max_by(|left, right| left.1.total_cmp(right.1))
        .map(|(index, _)| index)
}

fn digest_hex(bytes: &[u8]) -> String {
    content_digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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
