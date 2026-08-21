//! `characterize` command artifact flow.

use std::path::Path;

use pilotage_input::{CalibrationCandidate, canonical_candidate_digest, characterize_capture};

use crate::artifact_file::{
    self, MAX_CANDIDATE_BYTES, MAX_CAPTURE_BYTES, MAX_CONTRACT_BYTES, MAX_PROFILE_BYTES,
    read_bounded,
};
use crate::error::ProbeError;
use crate::output::print_line;

/// Creates a reviewable candidate from one capture and baseline profile.
///
/// # Errors
///
/// Returns [`ProbeError`] when an input, analysis, serialization, or output
/// operation fails.
pub fn run(contract: &Path, capture: &Path, profile: &Path, out: &Path) -> Result<(), ProbeError> {
    let contract_bytes = read_bounded(contract, MAX_CONTRACT_BYTES)?;
    let capture_bytes = read_bounded(capture, MAX_CAPTURE_BYTES)?;
    let profile_bytes = read_bounded(profile, MAX_PROFILE_BYTES)?;
    let candidate = characterize_capture(&contract_bytes, &capture_bytes, &profile_bytes)
        .map_err(|source| ProbeError::Characterization { source })?;
    let candidate_digest = canonical_candidate_digest(&candidate)
        .map_err(|source| ProbeError::CandidateDigest { source })?;
    write_candidate(out, &candidate, MAX_CANDIDATE_BYTES)?;
    print_line(&format!("Canonical candidate digest: {candidate_digest}"));
    Ok(())
}

fn write_candidate(
    out: &Path,
    candidate: &CalibrationCandidate,
    limit: usize,
) -> Result<(), ProbeError> {
    let mut bytes = serde_json::to_vec_pretty(candidate)
        .map_err(|source| ProbeError::CaptureSerialize { source })?;
    bytes.push(b'\n');
    artifact_file::write_new_bounded(out, &bytes, limit)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::write_candidate;
    use pilotage_input::characterize_capture;

    const CONTRACT: &[u8] = include_bytes!("../fixtures/browser-source-contract.json");
    const CAPTURE: &[u8] = include_bytes!("../fixtures/browser-capture.json");
    const PROFILE: &[u8] = include_bytes!("../fixtures/browser-profile.json");

    #[test]
    fn candidate_limit_is_checked_before_output_creation() {
        let candidate = characterize_capture(CONTRACT, CAPTURE, PROFILE).expect("candidate");
        let directory = tempfile::tempdir().expect("temporary directory");
        let out = directory.path().join("candidate.json");
        assert!(write_candidate(&out, &candidate, 1).is_err());
        assert!(!out.exists());
    }
}
