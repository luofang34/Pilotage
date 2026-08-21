//! Deterministic characterization of one portable HID capture.

use std::collections::HashMap;

use pilotage_input::{
    AxisCalibration, AxisCharacterization, AxisConfig, CALIBRATION_CANDIDATE_SCHEMA_VERSION,
    CHARACTERIZATION_CAPTURE_SCHEMA_VERSION, CalibrationCandidate, CaptureSample, CaptureSegment,
    CaptureSegmentKind, CharacterizationCapture, DeadzoneEvidenceStatus, DeviceProfile,
    TimestampSource, TimingCharacterization, content_digest, parse_profile_bytes,
};

use crate::error::ProbeError;

mod statistics;

use statistics::{CenterStats, center_statistics, characterize_timing, sample_time};

const MAX_CROSS_AXIS_COUPLING: f32 = 0.2;
const MIN_EXCURSION: f32 = 1.0e-4;
const MIN_CENTERED_SIDE_FRACTION: f32 = 0.1;
const MIN_DEADZONE: f32 = 0.002;
const MAX_DEADZONE: f32 = 0.2;

#[derive(Debug)]
struct MovementResult {
    axis: AxisCharacterization,
    selected_source: usize,
}

#[derive(Debug)]
struct AxisEvidence<'a> {
    logical: &'a str,
    source_index: usize,
    movement: &'a [&'a CaptureSample],
    idle: &'a [&'a CaptureSample],
    center: CenterStats,
    coupling: f32,
    timing_confidence: f32,
    deadzone_status: DeadzoneEvidenceStatus,
    timestamp_source: TimestampSource,
    requires_two_sided_range: bool,
}

/// Creates a calibration candidate from exact capture and baseline bytes.
///
/// # Errors
///
/// Returns [`ProbeError`] when the capture, profile, timing, segment layout,
/// or physical movement assignment is invalid.
pub fn characterize(
    capture_bytes: &[u8],
    capture: &CharacterizationCapture,
    baseline_profile_bytes: &[u8],
) -> Result<CalibrationCandidate, ProbeError> {
    let profile = parse_profile_bytes(baseline_profile_bytes)
        .map_err(|source| ProbeError::Profile { source })?;
    validate_capture(capture, &profile)?;
    let idle = segment_samples(capture, idle_segment(capture)?)?;
    let center_stats = center_statistics(&idle, capture.timestamp_source)?;
    let timing = characterize_timing(capture)?;
    let mut axes = Vec::new();
    let mut selected = HashMap::new();
    for segment in movement_segments(capture) {
        let result =
            characterize_movement(capture, &profile, segment, &idle, &center_stats, &timing)?;
        if let Some(first_logical) =
            selected.insert(result.selected_source, result.axis.logical.clone())
        {
            return Err(ProbeError::DuplicateMovement {
                first_logical,
                second_logical: result.axis.logical,
                source_index: result.selected_source,
            });
        }
        axes.push(result.axis);
    }
    if axes.is_empty() {
        return invalid("the capture has no movement segment");
    }
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
        timing,
        deadzone_evidence: capture.deadzone_evidence.clone(),
        axes,
        sample_count: count(capture.samples.len()),
        confidence,
    })
}

fn validate_capture(
    capture: &CharacterizationCapture,
    profile: &DeviceProfile,
) -> Result<(), ProbeError> {
    if capture.schema_version != CHARACTERIZATION_CAPTURE_SCHEMA_VERSION {
        return invalid("unsupported capture schema");
    }
    if capture.device.vendor_id != profile.device.vendor_id
        || capture.device.product_id != profile.device.product_id
    {
        return invalid("capture and baseline profile device identities differ");
    }
    let axis_count = capture
        .samples
        .first()
        .map(|sample| sample.axes.len())
        .ok_or_else(|| invalid_error("the capture has no samples"))?;
    if axis_count == 0 {
        return invalid("samples have no axes");
    }
    validate_samples(capture, axis_count)?;
    validate_segments(capture)?;
    validate_deadzone_evidence(capture)
}

fn validate_samples(
    capture: &CharacterizationCapture,
    axis_count: usize,
) -> Result<(), ProbeError> {
    for sample in &capture.samples {
        if sample.axes.len() != axis_count || sample.axes.iter().any(|value| !value.is_finite()) {
            return invalid("sample axes are non-finite or have different lengths");
        }
        if capture.timestamp_source == TimestampSource::Source && sample.source_at_us.is_none() {
            return invalid("the selected source clock is missing from a sample");
        }
    }
    for pair in capture.samples.windows(2) {
        if pair[1].sequence != pair[0].sequence.wrapping_add(1)
            || sample_time(&pair[1], capture.timestamp_source)
                <= sample_time(&pair[0], capture.timestamp_source)
        {
            return invalid("sample sequences and selected timestamps must increase");
        }
    }
    Ok(())
}

fn validate_segments(capture: &CharacterizationCapture) -> Result<(), ProbeError> {
    if capture.segments.is_empty() {
        return invalid("the capture has no segments");
    }
    let first = capture.samples.first().map(|sample| sample.sequence);
    let last = capture.samples.last().map(|sample| sample.sequence);
    for segment in &capture.segments {
        if segment.start_sequence > segment.end_sequence
            || first.is_none_or(|value| segment.start_sequence < value)
            || last.is_none_or(|value| segment.end_sequence > value)
        {
            return invalid("a segment range is outside the sample sequence");
        }
    }
    for pair in capture.segments.windows(2) {
        if pair[1].start_sequence <= pair[0].end_sequence {
            return invalid("capture segments overlap or are not in order");
        }
    }
    Ok(())
}

fn validate_deadzone_evidence(capture: &CharacterizationCapture) -> Result<(), ProbeError> {
    use pilotage_input::{
        DeadzoneEvidenceMethod as Method, DeadzoneEvidenceStatus as Status, SamplingSource,
    };
    let evidence = &capture.deadzone_evidence;
    let raw_source = matches!(
        capture.source,
        SamplingSource::AppleHid | SamplingSource::NativeHid | SamplingSource::Synthetic
    );
    let valid = match (evidence.status, evidence.method, evidence.sample_count) {
        (Status::Unknown, Method::Unmeasured, 0) => true,
        (Status::NotObserved, Method::RawHidReports, count) => raw_source && count > 0,
        (Status::Observed | Status::NotObserved, Method::PairedNativeAndPlatform, count) => {
            count > 0
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        invalid("platform dead-zone evidence is inconsistent")
    }
}

fn idle_segment(capture: &CharacterizationCapture) -> Result<&CaptureSegment, ProbeError> {
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
) -> Result<Vec<&'a CaptureSample>, ProbeError> {
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
) -> Result<MovementResult, ProbeError> {
    let (logical, positive_first) = movement_name(segment)?;
    if !positive_first {
        return invalid("a movement segment must move the positive direction first");
    }
    let samples = segment_samples(capture, segment)?;
    let baseline_axis = profile
        .axes
        .iter()
        .find(|axis| axis.logical == logical)
        .ok_or_else(|| invalid_error("movement logical name is not in the baseline profile"))?;
    let peaks = peak_excursions(&samples, centers);
    let selected = max_index(&peaks).ok_or_else(|| invalid_error("movement has no axes"))?;
    let primary = peaks[selected];
    let second = peaks
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != selected)
        .map(|(_, value)| *value)
        .fold(0.0f32, f32::max);
    let coupling = if primary > 0.0 { second / primary } else { 1.0 };
    let threshold = (centers[selected].noise * 8.0).max(MIN_EXCURSION);
    if primary < threshold || coupling > MAX_CROSS_AXIS_COUPLING {
        return Err(ProbeError::AmbiguousMovement {
            logical: logical.to_owned(),
            source_index: selected,
            coupling,
        });
    }
    let axis = build_axis(AxisEvidence {
        logical,
        source_index: selected,
        movement: &samples,
        idle,
        center: centers[selected],
        coupling,
        timing_confidence: timing.confidence,
        deadzone_status: capture.deadzone_evidence.status,
        timestamp_source: capture.timestamp_source,
        requires_two_sided_range: baseline_has_centered_range(baseline_axis),
    })?;
    Ok(MovementResult {
        axis,
        selected_source: selected,
    })
}

fn build_axis(evidence: AxisEvidence<'_>) -> Result<AxisCharacterization, ProbeError> {
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
    if evidence.requires_two_sided_range
        && !has_significant_sides(
            observed_min,
            evidence.center.center,
            observed_max,
            evidence.center.noise,
        )
    {
        return Err(ProbeError::IncompleteMovement {
            logical: evidence.logical.to_owned(),
            source_index: evidence.source_index,
        });
    }
    let direction_threshold = (evidence.center.noise * 8.0).max(span * 0.05);
    let first_direction = values
        .iter()
        .map(|value| *value - evidence.center.center)
        .find(|excursion| excursion.abs() >= direction_threshold)
        .ok_or_else(|| invalid_error("movement has no directional excursion"))?;
    let calibration =
        nondegenerate_calibration(observed_min, evidence.center.center, observed_max, span);
    let deadzone = proposed_deadzone(
        &evidence.deadzone_status,
        evidence.center,
        calibration,
        evidence.idle,
        evidence.timestamp_source,
    );
    let coverage = side_coverage(evidence.center.center, observed_min, observed_max, span);
    let sample_confidence =
        (evidence.movement.len().min(evidence.idle.len()) as f32 / 12.0).clamp(0.0, 1.0);
    let uniqueness = (1.0 - evidence.coupling).clamp(0.0, 1.0);
    let confidence = evidence
        .timing_confidence
        .min(sample_confidence * uniqueness * coverage);
    Ok(AxisCharacterization {
        logical: evidence.logical.to_owned(),
        source_index: evidence.source_index,
        invert: first_direction.is_sign_negative(),
        calibration,
        observed_min,
        observed_center: evidence.center.center,
        observed_max,
        center_noise: evidence.center.noise,
        center_drift_per_second: evidence.center.drift_per_second,
        center_behavior: evidence.center.behavior,
        cross_axis_coupling: evidence.coupling,
        proposed_deadzone: deadzone,
        idle_sample_count: count(evidence.idle.len()),
        movement_sample_count: count(evidence.movement.len()),
        confidence,
    })
}

fn nondegenerate_calibration(
    observed_min: f32,
    center: f32,
    observed_max: f32,
    span: f32,
) -> AxisCalibration {
    let padding = (span * 1.0e-6).max(f32::EPSILON * center.abs().max(1.0));
    AxisCalibration {
        min: if observed_min < center {
            observed_min
        } else {
            center - padding
        },
        center,
        max: if observed_max > center {
            observed_max
        } else {
            center + padding
        },
    }
}

fn proposed_deadzone(
    status: &DeadzoneEvidenceStatus,
    stats: CenterStats,
    calibration: AxisCalibration,
    idle: &[&CaptureSample],
    timestamp_source: TimestampSource,
) -> f32 {
    if *status != DeadzoneEvidenceStatus::NotObserved {
        return 0.0;
    }
    let idle_seconds = idle.last().zip(idle.first()).map_or(0.0, |(last, first)| {
        sample_time(last, timestamp_source).saturating_sub(sample_time(first, timestamp_source))
            as f32
            / 1_000_000.0
    });
    let disturbance = (stats.noise * 4.0).max(stats.drift_per_second * idle_seconds);
    let half_span = (calibration.center - calibration.min)
        .min(calibration.max - calibration.center)
        .max(MIN_EXCURSION);
    let normalized = disturbance / half_span;
    if normalized < MIN_DEADZONE {
        0.0
    } else {
        normalized.clamp(MIN_DEADZONE, MAX_DEADZONE)
    }
}

fn baseline_has_centered_range(axis: &AxisConfig) -> bool {
    let lower = axis.calibration.center - axis.calibration.min;
    let upper = axis.calibration.max - axis.calibration.center;
    let span = lower + upper;
    lower >= span * MIN_CENTERED_SIDE_FRACTION && upper >= span * MIN_CENTERED_SIDE_FRACTION
}

fn has_significant_sides(minimum: f32, center: f32, maximum: f32, noise: f32) -> bool {
    let lower = center - minimum;
    let upper = maximum - center;
    let span = lower + upper;
    let threshold = (span * MIN_CENTERED_SIDE_FRACTION)
        .max(noise * 8.0)
        .max(MIN_EXCURSION);
    lower >= threshold && upper >= threshold
}

fn side_coverage(center: f32, minimum: f32, maximum: f32, span: f32) -> f32 {
    let smaller = (center - minimum).min(maximum - center);
    if smaller >= span * MIN_CENTERED_SIDE_FRACTION {
        1.0
    } else {
        0.9
    }
}

fn movement_name(segment: &CaptureSegment) -> Result<(&str, bool), ProbeError> {
    match &segment.action {
        CaptureSegmentKind::Movement {
            logical,
            positive_first,
        } if !logical.is_empty() => Ok((logical, *positive_first)),
        CaptureSegmentKind::Movement { .. } => invalid("movement logical name is empty"),
        CaptureSegmentKind::Idle => invalid("expected a movement segment"),
    }
}

fn peak_excursions(samples: &[&CaptureSample], centers: &[CenterStats]) -> Vec<f32> {
    centers
        .iter()
        .enumerate()
        .map(|(axis, stats)| {
            samples
                .iter()
                .map(|sample| (sample.axes[axis] - stats.center).abs())
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

fn invalid<T>(detail: &str) -> Result<T, ProbeError> {
    Err(invalid_error(detail))
}

fn invalid_error(detail: &str) -> ProbeError {
    ProbeError::InvalidCapture {
        detail: detail.to_owned(),
    }
}

#[cfg(test)]
mod tests;
