//! `promote` command with digest-bound operator confirmation.

use std::path::Path;

use pilotage_input::{CalibrationCandidate, PromotionConfirmation, promote_calibration_candidate};

use crate::artifact_file::{
    self, MAX_CANDIDATE_BYTES, MAX_CAPTURE_BYTES, MAX_CONTRACT_BYTES, MAX_PROFILE_BYTES,
    read_bounded,
};
use crate::error::ProbeError;

/// Promotes one candidate into a new profile artifact.
///
/// # Errors
///
/// Returns [`ProbeError`] when an input, confirmation, promotion,
/// serialization, or output operation fails.
pub fn run(
    contract: &Path,
    capture: &Path,
    candidate: &Path,
    profile: &Path,
    out: &Path,
    confirmed_source_digest: &str,
    confirmed_candidate_digest: &str,
) -> Result<(), ProbeError> {
    let contract_bytes = read_bounded(contract, MAX_CONTRACT_BYTES)?;
    let capture_bytes = read_bounded(capture, MAX_CAPTURE_BYTES)?;
    let candidate_bytes = read_bounded(candidate, MAX_CANDIDATE_BYTES)?;
    let profile_bytes = read_bounded(profile, MAX_PROFILE_BYTES)?;
    let parsed: CalibrationCandidate =
        serde_json::from_slice(&candidate_bytes).map_err(|source| ProbeError::ArtifactParse {
            path: candidate.to_path_buf(),
            source,
        })?;
    let confirmation = PromotionConfirmation {
        source_capture_digest: confirmed_source_digest.to_owned(),
        candidate_digest: confirmed_candidate_digest.to_owned(),
    };
    let promoted = promote_calibration_candidate(
        &contract_bytes,
        &capture_bytes,
        &profile_bytes,
        &parsed,
        &confirmation,
    )
    .map_err(|source| ProbeError::Promotion { source })?;
    let mut bytes = serde_json::to_vec_pretty(&promoted)
        .map_err(|source| ProbeError::CaptureSerialize { source })?;
    bytes.push(b'\n');
    artifact_file::write_new_bounded(out, &bytes, MAX_PROFILE_BYTES)
}
