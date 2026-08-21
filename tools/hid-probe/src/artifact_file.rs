//! Create-new storage for reviewed HID artifacts.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use crate::error::ProbeError;

pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<(), ProbeError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| ProbeError::CaptureWrite {
            path: path.to_path_buf(),
            source,
        })?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|source| ProbeError::CaptureWrite {
            path: path.to_path_buf(),
            source,
        })
}

#[cfg(test)]
#[path = "artifact_file/tests.rs"]
mod tests;
