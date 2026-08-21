//! Explicit and restricted calibration candidate promotion.

use super::analysis::{AnalysisError, characterize_capture};
use super::candidate::{CALIBRATION_CANDIDATE_SCHEMA_VERSION, CalibrationCandidate};
use crate::{
    DeviceProfile, ProfileError, content_digest, parse_profile_bytes, validate_physical_axis_config,
};

mod validation;

use validation::{validate_assignments, validate_device, validate_header};

/// The minimum candidate confidence that promotion accepts.
const MIN_PROMOTION_CONFIDENCE: f32 = 0.8;

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
    /// Exact source evidence could not produce a calibration candidate.
    #[error("failed to regenerate the calibration candidate: {source}")]
    Analysis {
        /// The exact-capture analysis error.
        #[source]
        source: AnalysisError,
    },
    /// The candidate uses an unsupported schema.
    #[error("unsupported calibration candidate schema {found}; expected {expected}")]
    UnsupportedSchema {
        /// The candidate schema.
        found: u32,
        /// The supported schema.
        expected: u32,
    },
    /// The candidate came from a source that cannot produce qualified evidence.
    #[error("sampling source {sampling_source:?} cannot be promoted")]
    UnsupportedPromotionSource {
        /// The refused sampling source.
        sampling_source: crate::SamplingSource,
    },
    /// The candidate does not identify one specific physical device.
    #[error("device {vendor_id:04x}:{product_id:04x} cannot be promoted")]
    UnsupportedDeviceIdentity {
        /// The refused USB vendor ID.
        vendor_id: u16,
        /// The refused USB product ID.
        product_id: u16,
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
    /// The supplied candidate differs from the exact regenerated candidate.
    #[error(
        "candidate digest {supplied} does not match regenerated candidate digest {regenerated}"
    )]
    CandidateEvidenceMismatch {
        /// The supplied canonical candidate digest.
        supplied: String,
        /// The exact regenerated candidate digest.
        regenerated: String,
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
    source_contract_bytes: &[u8],
    source_capture_bytes: &[u8],
    baseline_profile_bytes: &[u8],
    candidate: &CalibrationCandidate,
    confirmation: &PromotionConfirmation,
) -> Result<DeviceProfile, CharacterizationError> {
    let regenerated = characterize_capture(
        source_contract_bytes,
        source_capture_bytes,
        baseline_profile_bytes,
    )
    .map_err(|source| CharacterizationError::Analysis { source })?;
    let supplied_bytes = canonical_candidate_bytes(candidate)?;
    let regenerated_bytes = canonical_candidate_bytes(&regenerated)?;
    if supplied_bytes != regenerated_bytes {
        return Err(CharacterizationError::CandidateEvidenceMismatch {
            supplied: digest_hex(&supplied_bytes),
            regenerated: digest_hex(&regenerated_bytes),
        });
    }
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
    let bytes = canonical_candidate_bytes(candidate)?;
    Ok(digest_hex(&bytes))
}

fn canonical_candidate_bytes(
    candidate: &CalibrationCandidate,
) -> Result<Vec<u8>, CharacterizationError> {
    serde_json::to_vec(candidate)
        .map_err(|source| CharacterizationError::CandidateSerialize { source })
}

fn digest_hex(bytes: &[u8]) -> String {
    content_digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
