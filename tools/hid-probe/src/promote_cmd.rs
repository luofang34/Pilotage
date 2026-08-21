//! `promote` command with digest-bound operator confirmation.

use std::path::Path;

use pilotage_input::{CalibrationCandidate, PromotionConfirmation, promote_calibration_candidate};

use crate::artifact_file;
use crate::error::ProbeError;

/// Promotes one candidate into a new profile artifact.
///
/// # Errors
///
/// Returns [`ProbeError`] when an input, confirmation, promotion,
/// serialization, or output operation fails.
pub fn run(
    candidate: &Path,
    profile: &Path,
    out: &Path,
    confirmed_source_digest: &str,
    confirmed_candidate_digest: &str,
) -> Result<(), ProbeError> {
    let candidate_bytes = read(candidate)?;
    let profile_bytes = read(profile)?;
    let parsed: CalibrationCandidate =
        serde_json::from_slice(&candidate_bytes).map_err(|source| ProbeError::ArtifactParse {
            path: candidate.to_path_buf(),
            source,
        })?;
    let confirmation = PromotionConfirmation {
        source_capture_digest: confirmed_source_digest.to_owned(),
        candidate_digest: confirmed_candidate_digest.to_owned(),
    };
    let promoted = promote_calibration_candidate(&profile_bytes, &parsed, &confirmation)
        .map_err(|source| ProbeError::Promotion { source })?;
    let json = serde_json::to_string_pretty(&promoted)
        .map_err(|source| ProbeError::CaptureSerialize { source })?;
    artifact_file::write_new(out, format!("{json}\n").as_bytes())
}

fn read(path: &Path) -> Result<Vec<u8>, ProbeError> {
    std::fs::read(path).map_err(|source| ProbeError::ArtifactRead {
        path: path.to_path_buf(),
        source,
    })
}
