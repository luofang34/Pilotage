//! Independent checks before a regenerated candidate changes a profile.

use std::collections::HashSet;

use crate::{
    AxisCalibration, CalibrationCandidate, DeadzoneEvidenceMethod, DeadzoneEvidenceStatus,
    DeviceProfile, SamplingSource, TimingObservation,
};

use super::{
    CALIBRATION_CANDIDATE_SCHEMA_VERSION, CharacterizationError, MIN_PROMOTION_CONFIDENCE,
    PromotionConfirmation, canonical_candidate_digest, digest_hex,
};

pub(super) fn validate_header(
    baseline: &[u8],
    candidate: &CalibrationCandidate,
    confirmation: &PromotionConfirmation,
) -> Result<(), CharacterizationError> {
    if candidate.schema_version != CALIBRATION_CANDIDATE_SCHEMA_VERSION {
        return Err(CharacterizationError::UnsupportedSchema {
            found: candidate.schema_version,
            expected: CALIBRATION_CANDIDATE_SCHEMA_VERSION,
        });
    }
    validate_digest("source_capture_digest", &candidate.source_capture_digest)?;
    validate_digest("source_contract_digest", &candidate.source_contract_digest)?;
    validate_digest(
        "baseline_profile_digest",
        &candidate.baseline_profile_digest,
    )?;
    validate_digest("candidate_digest", &confirmation.candidate_digest)?;
    if confirmation.source_capture_digest != candidate.source_capture_digest {
        return Err(CharacterizationError::ConfirmationMismatch {
            confirmed: confirmation.source_capture_digest.clone(),
            candidate: candidate.source_capture_digest.clone(),
        });
    }
    let actual_candidate_digest = canonical_candidate_digest(candidate)?;
    if confirmation.candidate_digest != actual_candidate_digest {
        return Err(CharacterizationError::CandidateConfirmationMismatch {
            confirmed: confirmation.candidate_digest.clone(),
            actual: actual_candidate_digest,
        });
    }
    let actual = digest_hex(baseline);
    if actual != candidate.baseline_profile_digest {
        return Err(CharacterizationError::BaselineDigestMismatch {
            actual,
            expected: candidate.baseline_profile_digest.clone(),
        });
    }
    validate_evidence(candidate)
}

fn validate_evidence(candidate: &CalibrationCandidate) -> Result<(), CharacterizationError> {
    if candidate.source == SamplingSource::Synthetic {
        return Err(CharacterizationError::UnsupportedPromotionSource {
            sampling_source: candidate.source,
        });
    }
    if candidate.device.vendor_id == 0 && candidate.device.product_id == 0 {
        return Err(CharacterizationError::UnsupportedDeviceIdentity {
            vendor_id: candidate.device.vendor_id,
            product_id: candidate.device.product_id,
        });
    }
    if candidate.sample_count == 0 || candidate.axes.is_empty() {
        return Err(CharacterizationError::EmptyEvidence);
    }
    let derived_confidence = candidate
        .axes
        .iter()
        .map(|axis| axis.confidence)
        .chain(core::iter::once(candidate.timing.confidence))
        .fold(1.0f32, f32::min);
    if !candidate.confidence.is_finite()
        || !(MIN_PROMOTION_CONFIDENCE..=1.0).contains(&candidate.confidence)
        || candidate.confidence.to_bits() != derived_confidence.to_bits()
    {
        return Err(CharacterizationError::LowConfidence {
            confidence: candidate.confidence,
            minimum: MIN_PROMOTION_CONFIDENCE,
        });
    }
    validate_timing(candidate)?;
    validate_deadzone_evidence(candidate)
}

fn validate_timing(candidate: &CalibrationCandidate) -> Result<(), CharacterizationError> {
    let observation_matches_source = match candidate.source {
        SamplingSource::BrowserGamepad => {
            candidate.timing.observation == TimingObservation::PolledStateUpdates
                && candidate.timing.dropped_report_count.is_none()
        }
        SamplingSource::AppleHid | SamplingSource::NativeHid => {
            candidate.timing.observation == TimingObservation::ReportCallbacks
                && candidate.timing.dropped_report_count.is_some()
        }
        SamplingSource::Synthetic => false,
    };
    if candidate.timing.sample_count != candidate.sample_count
        || candidate.timing.sample_count < 2
        || !candidate.timing.median_period_us.is_finite()
        || candidate.timing.median_period_us <= 0.0
        || !candidate.timing.jitter_mad_us.is_finite()
        || candidate.timing.jitter_mad_us < 0.0
        || !(MIN_PROMOTION_CONFIDENCE..=1.0).contains(&candidate.timing.confidence)
        || !observation_matches_source
    {
        return Err(CharacterizationError::InvalidTimingEvidence);
    }
    Ok(())
}

pub(super) fn validate_device(
    profile: &DeviceProfile,
    candidate: &CalibrationCandidate,
) -> Result<(), CharacterizationError> {
    if profile.device.vendor_id == candidate.device.vendor_id
        && profile.device.product_id == candidate.device.product_id
    {
        return Ok(());
    }
    Err(CharacterizationError::DeviceMismatch {
        candidate_vendor: candidate.device.vendor_id,
        candidate_product: candidate.device.product_id,
        profile_vendor: profile.device.vendor_id,
        profile_product: profile.device.product_id,
    })
}

pub(super) fn validate_assignments(
    profile: &DeviceProfile,
    candidate: &CalibrationCandidate,
) -> Result<(), CharacterizationError> {
    let mut logicals = HashSet::new();
    let mut sources = HashSet::new();
    for axis in &candidate.axes {
        if !profile
            .axes
            .iter()
            .any(|baseline| baseline.logical == axis.logical)
        {
            return Err(CharacterizationError::MissingAxis {
                logical: axis.logical.clone(),
            });
        }
        if !valid_axis_evidence(candidate, axis) {
            return Err(CharacterizationError::InvalidAxisEvidence {
                logical: axis.logical.clone(),
            });
        }
        if !(MIN_PROMOTION_CONFIDENCE..=1.0).contains(&axis.confidence) {
            return Err(CharacterizationError::AxisLowConfidence {
                logical: axis.logical.clone(),
                confidence: axis.confidence,
                minimum: MIN_PROMOTION_CONFIDENCE,
            });
        }
        if !logicals.insert(axis.logical.as_str()) || !sources.insert(axis.source_index) {
            return Err(CharacterizationError::DuplicateAssignment {
                logical: axis.logical.clone(),
                source_index: axis.source_index,
            });
        }
    }
    Ok(())
}

fn valid_axis_evidence(
    candidate: &CalibrationCandidate,
    axis: &crate::AxisCharacterization,
) -> bool {
    let derived_calibration = axis.derived_calibration();
    let derived_deadzone = axis.derived_deadzone(candidate.deadzone_evidence.status);
    let derived_confidence = axis.derived_confidence(candidate.timing.confidence);
    axis.source_range.source_index == axis.source_index
        && axis.idle_sample_count > 0
        && axis.idle_sample_count <= candidate.sample_count
        && axis.movement_sample_count > 0
        && axis.movement_sample_count <= candidate.sample_count
        && axis.idle_duration_us > 0
        && axis.observed_min.is_finite()
        && axis.observed_center.is_finite()
        && axis.observed_max.is_finite()
        && axis.observed_min <= axis.observed_center
        && axis.observed_center <= axis.observed_max
        && axis.center_noise.is_finite()
        && axis.center_noise >= 0.0
        && axis.center_drift_per_second.is_finite()
        && axis.center_drift_per_second >= 0.0
        && axis.first_direction_excursion.is_finite()
        && axis.first_direction_excursion != 0.0
        && axis.invert == axis.derived_invert()
        && axis.cross_axis_coupling.is_finite()
        && (0.0..=0.2).contains(&axis.cross_axis_coupling)
        && axis.has_required_range()
        && calibration_equal(axis.calibration, derived_calibration)
        && axis.proposed_deadzone.to_bits() == derived_deadzone.to_bits()
        && axis.confidence.to_bits() == derived_confidence.to_bits()
}

fn calibration_equal(left: AxisCalibration, right: AxisCalibration) -> bool {
    left.min.to_bits() == right.min.to_bits()
        && left.center.to_bits() == right.center.to_bits()
        && left.max.to_bits() == right.max.to_bits()
}

fn validate_deadzone_evidence(
    candidate: &CalibrationCandidate,
) -> Result<(), CharacterizationError> {
    let evidence = &candidate.deadzone_evidence;
    let raw_source = matches!(
        candidate.source,
        SamplingSource::AppleHid | SamplingSource::NativeHid
    );
    let consistent = match (evidence.status, evidence.method, evidence.sample_count) {
        (DeadzoneEvidenceStatus::Unknown, DeadzoneEvidenceMethod::Unmeasured, 0) => true,
        (DeadzoneEvidenceStatus::NotObserved, DeadzoneEvidenceMethod::RawHidReports, count) => {
            raw_source && count > 0 && count <= candidate.sample_count
        }
        _ => false,
    };
    let unsupported_deadzone = evidence.status != DeadzoneEvidenceStatus::NotObserved
        && candidate
            .axes
            .iter()
            .any(|axis| axis.proposed_deadzone != 0.0);
    if consistent && !unsupported_deadzone {
        Ok(())
    } else {
        Err(CharacterizationError::InvalidDeadzoneEvidence)
    }
}

fn validate_digest(field: &'static str, value: &str) -> Result<(), CharacterizationError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Ok(());
    }
    Err(CharacterizationError::InvalidDigest {
        field,
        value: value.to_owned(),
    })
}
