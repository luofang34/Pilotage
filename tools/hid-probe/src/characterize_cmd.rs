//! `characterize` command artifact flow.

use std::path::Path;

use pilotage_input::{CharacterizationCapture, canonical_candidate_digest};

use crate::analysis::characterize;
use crate::artifact_file;
use crate::error::ProbeError;
use crate::output::print_line;

/// Creates a reviewable candidate from one capture and baseline profile.
///
/// # Errors
///
/// Returns [`ProbeError`] when an input, analysis, serialization, or output
/// operation fails.
pub fn run(capture: &Path, profile: &Path, out: &Path) -> Result<(), ProbeError> {
    let capture_bytes = read(capture)?;
    let profile_bytes = read(profile)?;
    let parsed: CharacterizationCapture =
        serde_json::from_slice(&capture_bytes).map_err(|source| ProbeError::ArtifactParse {
            path: capture.to_path_buf(),
            source,
        })?;
    let candidate = characterize(&capture_bytes, &parsed, &profile_bytes)?;
    let candidate_digest = canonical_candidate_digest(&candidate)
        .map_err(|source| ProbeError::CandidateDigest { source })?;
    let json = serde_json::to_string_pretty(&candidate)
        .map_err(|source| ProbeError::CaptureSerialize { source })?;
    artifact_file::write_new(out, format!("{json}\n").as_bytes())?;
    print_line(&format!("Canonical candidate digest: {candidate_digest}"));
    Ok(())
}

fn read(path: &Path) -> Result<Vec<u8>, ProbeError> {
    std::fs::read(path).map_err(|source| ProbeError::ArtifactRead {
        path: path.to_path_buf(),
        source,
    })
}
