//! Explicit and restricted calibration candidate promotion.

use std::collections::HashSet;

use super::candidate::{CALIBRATION_CANDIDATE_SCHEMA_VERSION, CalibrationCandidate};
use super::capture::{DeadzoneEvidenceMethod, DeadzoneEvidenceStatus, SamplingSource};
use crate::{
    AxisConfig, DeviceProfile, ProfileError, content_digest, parse_profile_bytes,
    validate_physical_axis_config,
};

/// The minimum candidate confidence that promotion accepts.
const MIN_PROMOTION_CONFIDENCE: f32 = 0.8;
const MIN_CENTERED_SIDE_FRACTION: f32 = 0.1;
const CENTER_NOISE_MULTIPLIER: f32 = 8.0;
const MIN_EXCURSION: f32 = 1.0e-4;

/// Operator confirmation for one source capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotionConfirmation {
    /// The source digest that the operator reviewed and accepted.
    pub source_capture_digest: String,
    /// The canonical candidate digest that the operator reviewed and accepted.
    pub candidate_digest: String,
}

/// Errors from calibration candidate promotion.
#[derive(Debug, thiserror::Error)]
pub enum CharacterizationError {
    /// The candidate uses an unsupported schema.
    #[error("unsupported calibration candidate schema {found}; expected {expected}")]
    UnsupportedSchema {
        /// The candidate schema.
        found: u32,
        /// The supported schema.
        expected: u32,
    },
    /// A digest does not use lowercase SHA-256 hexadecimal form.
    #[error("{field} is not a lowercase SHA-256 digest: {value}")]
    InvalidDigest {
        /// The digest field.
        field: &'static str,
        /// The invalid value.
        value: String,
    },
    /// The operator did not confirm the candidate's source capture.
    #[error("confirmed source digest {confirmed} does not match candidate digest {candidate}")]
    ConfirmationMismatch {
        /// The digest supplied at promotion.
        confirmed: String,
        /// The digest bound into the candidate.
        candidate: String,
    },
    /// The operator did not confirm the exact canonical candidate.
    #[error("confirmed candidate digest {confirmed} does not match actual digest {actual}")]
    CandidateConfirmationMismatch {
        /// The digest supplied at promotion.
        confirmed: String,
        /// The digest of the supplied canonical candidate.
        actual: String,
    },
    /// The baseline profile bytes do not match the candidate lineage.
    #[error("baseline profile digest {actual} does not match candidate digest {expected}")]
    BaselineDigestMismatch {
        /// The digest of the supplied profile bytes.
        actual: String,
        /// The digest bound into the candidate.
        expected: String,
    },
    /// The profile is invalid.
    #[error("baseline or promoted device profile is invalid: {source}")]
    Profile {
        /// The profile validation error.
        #[source]
        source: ProfileError,
    },
    /// The candidate targets a different USB identity.
    #[error(
        "candidate device {candidate_vendor:04x}:{candidate_product:04x} does not match profile device {profile_vendor:04x}:{profile_product:04x}"
    )]
    DeviceMismatch {
        /// Candidate vendor ID.
        candidate_vendor: u16,
        /// Candidate product ID.
        candidate_product: u16,
        /// Profile vendor ID.
        profile_vendor: u16,
        /// Profile product ID.
        profile_product: u16,
    },
    /// The candidate confidence is below the promotion floor.
    #[error("candidate confidence {confidence} is below {minimum}")]
    LowConfidence {
        /// The rejected confidence.
        confidence: f32,
        /// The required confidence.
        minimum: f32,
    },
    /// One axis confidence is outside the accepted range.
    #[error("axis {logical} confidence {confidence} is outside [{minimum}, 1]")]
    AxisLowConfidence {
        /// The rejected logical axis.
        logical: String,
        /// The rejected confidence.
        confidence: f32,
        /// The required confidence.
        minimum: f32,
    },
    /// The candidate has no samples or no axis proposals.
    #[error("candidate evidence is empty")]
    EmptyEvidence,
    /// An axis proposal has no sample evidence or invalid measured values.
    #[error("candidate axis evidence is invalid for {logical}")]
    InvalidAxisEvidence {
        /// The invalid logical axis.
        logical: String,
    },
    /// Timing evidence does not match the candidate sample count.
    #[error("candidate timing evidence is invalid")]
    InvalidTimingEvidence,
    /// Platform dead-zone evidence is inconsistent with the sampling source.
    #[error("candidate platform dead-zone evidence is invalid")]
    InvalidDeadzoneEvidence,
    /// More than one candidate entry targets the same logical or source axis.
    #[error("candidate axis assignment is not unique for {logical} at source {source_index}")]
    DuplicateAssignment {
        /// The repeated logical name.
        logical: String,
        /// The repeated source index.
        source_index: usize,
    },
    /// The baseline has no axis with the candidate logical name.
    #[error("baseline profile has no axis named {logical}")]
    MissingAxis {
        /// The missing logical name.
        logical: String,
    },
    /// Serialization of canonical candidate bytes failed.
    #[error("failed to serialize canonical calibration candidate: {source}")]
    CandidateSerialize {
        /// The JSON serialization error.
        #[source]
        source: serde_json::Error,
    },
    /// Serialization of a validated promoted profile failed.
    #[error("failed to serialize promoted device profile: {source}")]
    ProfileSerialize {
        /// The JSON serialization error.
        #[source]
        source: serde_json::Error,
    },
}

impl From<ProfileError> for CharacterizationError {
    fn from(source: ProfileError) -> Self {
        Self::Profile { source }
    }
}

/// Applies only device mapping, calibration, inversion, and measured noise
/// suppression from a confirmed candidate.
///
/// The function preserves all response-curve values and all profile content
/// outside the selected physical axes. It increments the profile revision
/// with wrapping semantics.
///
/// # Errors
///
/// Returns [`CharacterizationError`] when lineage, confirmation, confidence,
/// device identity, uniqueness, or the promoted physical profile is invalid.
pub fn promote_calibration_candidate(
    baseline_profile_bytes: &[u8],
    candidate: &CalibrationCandidate,
    confirmation: &PromotionConfirmation,
) -> Result<DeviceProfile, CharacterizationError> {
    validate_header(baseline_profile_bytes, candidate, confirmation)?;
    let mut profile = parse_profile_bytes(baseline_profile_bytes)?;
    validate_device(&profile, candidate)?;
    validate_assignments(&profile, candidate)?;
    for proposal in &candidate.axes {
        let axis = profile
            .axes
            .iter_mut()
            .find(|axis| axis.logical == proposal.logical)
            .ok_or_else(|| CharacterizationError::MissingAxis {
                logical: proposal.logical.clone(),
            })?;
        let expo = axis.expo;
        axis.source_index = proposal.source_index;
        axis.invert = proposal.invert;
        axis.calibration = proposal.calibration;
        axis.deadzone = proposal.proposed_deadzone;
        axis.expo = expo;
        validate_physical_axis_config(axis)?;
    }
    profile.revision = profile.revision.wrapping_add(1);
    let bytes = serde_json::to_vec(&profile)
        .map_err(|source| CharacterizationError::ProfileSerialize { source })?;
    parse_profile_bytes(&bytes).map_err(Into::into)
}

/// Returns the SHA-256 digest of the candidate's canonical compact JSON bytes.
///
/// The schema uses structs and ordered lists. Its compact JSON encoding has
/// one stable field order.
///
/// # Errors
///
/// Returns [`CharacterizationError::CandidateSerialize`] when the candidate
/// contains a value that JSON cannot encode.
pub fn canonical_candidate_digest(
    candidate: &CalibrationCandidate,
) -> Result<String, CharacterizationError> {
    let bytes = serde_json::to_vec(candidate)
        .map_err(|source| CharacterizationError::CandidateSerialize { source })?;
    Ok(digest_hex(&bytes))
}

fn validate_header(
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
    if candidate.sample_count == 0 || candidate.axes.is_empty() {
        return Err(CharacterizationError::EmptyEvidence);
    }
    if !candidate.confidence.is_finite()
        || !(MIN_PROMOTION_CONFIDENCE..=1.0).contains(&candidate.confidence)
    {
        return Err(CharacterizationError::LowConfidence {
            confidence: candidate.confidence,
            minimum: MIN_PROMOTION_CONFIDENCE,
        });
    }
    if candidate.timing.sample_count != candidate.sample_count
        || candidate.timing.sample_count < 2
        || !candidate.timing.median_period_us.is_finite()
        || candidate.timing.median_period_us <= 0.0
        || !candidate.timing.jitter_mad_us.is_finite()
        || candidate.timing.jitter_mad_us < 0.0
        || !(MIN_PROMOTION_CONFIDENCE..=1.0).contains(&candidate.timing.confidence)
    {
        return Err(CharacterizationError::InvalidTimingEvidence);
    }
    validate_deadzone_evidence(candidate)?;
    Ok(())
}

fn validate_device(
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

fn validate_assignments(
    profile: &DeviceProfile,
    candidate: &CalibrationCandidate,
) -> Result<(), CharacterizationError> {
    let mut logicals = HashSet::new();
    let mut sources = HashSet::new();
    for axis in &candidate.axes {
        let baseline = profile
            .axes
            .iter()
            .find(|baseline| baseline.logical == axis.logical)
            .ok_or_else(|| CharacterizationError::MissingAxis {
                logical: axis.logical.clone(),
            })?;
        if axis.idle_sample_count == 0
            || axis.movement_sample_count == 0
            || !axis.observed_min.is_finite()
            || !axis.observed_center.is_finite()
            || !axis.observed_max.is_finite()
            || axis.observed_min > axis.observed_center
            || axis.observed_center > axis.observed_max
            || !axis.center_noise.is_finite()
            || axis.center_noise < 0.0
            || !axis.center_drift_per_second.is_finite()
            || axis.center_drift_per_second < 0.0
            || !axis.cross_axis_coupling.is_finite()
            || !(0.0..=0.2).contains(&axis.cross_axis_coupling)
            || (baseline_has_centered_range(baseline) && !candidate_has_significant_sides(axis))
        {
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

fn baseline_has_centered_range(axis: &AxisConfig) -> bool {
    let lower = axis.calibration.center - axis.calibration.min;
    let upper = axis.calibration.max - axis.calibration.center;
    let span = lower + upper;
    lower >= span * MIN_CENTERED_SIDE_FRACTION && upper >= span * MIN_CENTERED_SIDE_FRACTION
}

fn candidate_has_significant_sides(axis: &super::candidate::AxisCharacterization) -> bool {
    let lower = axis.observed_center - axis.observed_min;
    let upper = axis.observed_max - axis.observed_center;
    let span = lower + upper;
    let threshold = (span * MIN_CENTERED_SIDE_FRACTION)
        .max(axis.center_noise * CENTER_NOISE_MULTIPLIER)
        .max(MIN_EXCURSION);
    lower >= threshold && upper >= threshold
}

fn validate_deadzone_evidence(
    candidate: &CalibrationCandidate,
) -> Result<(), CharacterizationError> {
    let evidence = &candidate.deadzone_evidence;
    let raw_source = matches!(
        candidate.source,
        SamplingSource::AppleHid | SamplingSource::NativeHid | SamplingSource::Synthetic
    );
    let consistent = match (evidence.status, evidence.method, evidence.sample_count) {
        (DeadzoneEvidenceStatus::Unknown, DeadzoneEvidenceMethod::Unmeasured, 0) => true,
        (DeadzoneEvidenceStatus::NotObserved, DeadzoneEvidenceMethod::RawHidReports, count) => {
            raw_source && count > 0
        }
        (
            DeadzoneEvidenceStatus::Observed | DeadzoneEvidenceStatus::NotObserved,
            DeadzoneEvidenceMethod::PairedNativeAndPlatform,
            count,
        ) => count > 0,
        _ => false,
    };
    let double_deadzone = matches!(
        evidence.status,
        DeadzoneEvidenceStatus::Unknown | DeadzoneEvidenceStatus::Observed
    ) && candidate
        .axes
        .iter()
        .any(|axis| axis.proposed_deadzone != 0.0);
    if consistent && !double_deadzone {
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

fn digest_hex(bytes: &[u8]) -> String {
    content_digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
