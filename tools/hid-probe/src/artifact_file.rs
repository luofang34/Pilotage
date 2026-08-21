//! Create-new storage for reviewed HID artifacts.

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::Path;

use pilotage_input::MAX_CHARACTERIZATION_CAPTURE_BYTES;
use serde::Serialize;

use crate::error::ProbeError;

pub(crate) const MAX_CAPTURE_BYTES: usize = MAX_CHARACTERIZATION_CAPTURE_BYTES;
pub(crate) const MAX_CONTRACT_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_PROFILE_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_CANDIDATE_BYTES: usize = 4 * 1024 * 1024;

pub(crate) fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, ProbeError> {
    let file =
        OpenOptions::new()
            .read(true)
            .open(path)
            .map_err(|source| ProbeError::ArtifactRead {
                path: path.to_path_buf(),
                source,
            })?;
    let read_limit = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    let mut bytes = Vec::new();
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|source| ProbeError::ArtifactRead {
            path: path.to_path_buf(),
            source,
        })?;
    validate_artifact_byte_count(path, bytes.len(), limit)?;
    Ok(bytes)
}

pub(crate) fn validate_artifact_byte_count(
    path: &Path,
    actual: usize,
    limit: usize,
) -> Result<(), ProbeError> {
    if actual > limit {
        Err(ProbeError::ArtifactTooLarge {
            path: path.to_path_buf(),
            limit,
        })
    } else {
        Ok(())
    }
}

pub(crate) fn write_new_bounded(path: &Path, bytes: &[u8], limit: usize) -> Result<(), ProbeError> {
    validate_artifact_byte_count(path, bytes.len(), limit)?;
    write_new(path, bytes)
}

pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<(), ProbeError> {
    let mut file = open_new(path)?;
    file.write_all(bytes)
        .map_err(|source| ProbeError::CaptureWrite {
            path: path.to_path_buf(),
            source,
        })?;
    sync(path, &file)
}

pub(crate) fn write_new_json<T: Serialize>(path: &Path, value: &T) -> Result<(), ProbeError> {
    let mut file = open_new(path)?;
    serde_json::to_writer_pretty(&mut file, value).map_err(|source| ProbeError::CaptureStream {
        path: path.to_path_buf(),
        source,
    })?;
    file.write_all(b"\n")
        .map_err(|source| ProbeError::CaptureWrite {
            path: path.to_path_buf(),
            source,
        })?;
    sync(path, &file)
}

fn open_new(path: &Path) -> Result<std::fs::File, ProbeError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| ProbeError::CaptureWrite {
            path: path.to_path_buf(),
            source,
        })
}

fn sync(path: &Path, file: &std::fs::File) -> Result<(), ProbeError> {
    file.sync_all().map_err(|source| ProbeError::CaptureWrite {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
#[path = "artifact_file/tests.rs"]
mod tests;
