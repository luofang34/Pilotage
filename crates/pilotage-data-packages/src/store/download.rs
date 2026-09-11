use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

use sha2::{Digest, Sha256};

use crate::{
    Artifact, ContentDigest, PackageError, Release,
    error::{file_error, invalid},
};

use super::PackageStore;

/// One object that must be downloaded before installation.
#[derive(Debug, Clone)]
pub struct Download {
    /// Required object and its source.
    pub artifact: Artifact,
    /// Bytes already staged for this exact object.
    pub offset: u64,
}

/// Missing objects for one release.
#[derive(Debug, Clone)]
pub struct InstallPlan {
    /// Downloads, with identical content included once.
    pub downloads: Vec<Download>,
    /// Additional bytes needed to complete the staged objects.
    pub remaining_bytes: u64,
}

impl PackageStore {
    /// Plan an install against a caller-supplied storage allowance.
    pub fn plan_blocking(
        &self,
        release: &Release,
        available_bytes: u64,
    ) -> Result<InstallPlan, PackageError> {
        release.validate()?;
        let mut seen = BTreeSet::new();
        let mut plan = InstallPlan {
            downloads: Vec::new(),
            remaining_bytes: 0,
        };
        for artifact in &release.artifacts {
            if !seen.insert(&artifact.sha256) {
                continue;
            }
            let object = self.object_path(&artifact.sha256);
            if object.exists() {
                verify_file_blocking(&object, artifact)?;
                continue;
            }
            let partial = self.partial_path(&artifact.sha256);
            let offset = match fs::metadata(&partial) {
                Ok(metadata) => metadata.len(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
                Err(source) => return Err(file_error(partial, source)),
            };
            if offset > artifact.bytes {
                return Err(length_error(artifact, offset));
            }
            plan.remaining_bytes = plan
                .remaining_bytes
                .checked_add(artifact.bytes - offset)
                .ok_or_else(|| invalid("package_bytes", "overflow"))?;
            plan.downloads.push(Download {
                artifact: artifact.clone(),
                offset,
            });
        }
        if plan.remaining_bytes > available_bytes {
            return Err(PackageError::Space {
                required: plan.remaining_bytes,
                available: available_bytes,
            });
        }
        Ok(plan)
    }

    /// Append one bounded transfer chunk at its expected offset.
    pub fn append_blocking(
        &mut self,
        artifact: &Artifact,
        offset: u64,
        bytes: &[u8],
    ) -> Result<(), PackageError> {
        let path = self.partial_path(&artifact.sha256);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|s| file_error(&path, s))?;
        let actual = file.metadata().map_err(|s| file_error(&path, s))?.len();
        if actual != offset {
            return Err(invalid(
                "download_offset",
                format!("expected {actual}, got {offset}"),
            ));
        }
        let end = offset
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| invalid("download_length", "overflow"))?;
        if end > artifact.bytes {
            return Err(length_error(artifact, end));
        }
        file.write_all(bytes).map_err(|s| file_error(&path, s))?;
        file.sync_data().map_err(|s| file_error(path, s))
    }

    /// Delete a partial object so a failed or changed transfer can restart.
    pub fn discard_download_blocking(
        &mut self,
        digest: &ContentDigest,
    ) -> Result<(), PackageError> {
        let path = self.partial_path(digest);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(file_error(path, source)),
        }
    }

    pub(super) fn object_path(&self, digest: &ContentDigest) -> std::path::PathBuf {
        self.root.join("objects").join(digest.as_str())
    }

    pub(super) fn partial_path(&self, digest: &ContentDigest) -> std::path::PathBuf {
        self.root
            .join("staging")
            .join(format!("{}.part", digest.as_str()))
    }
}

pub(super) fn verify_file_blocking(path: &Path, artifact: &Artifact) -> Result<(), PackageError> {
    let mut file = File::open(path).map_err(|s| file_error(path, s))?;
    let length = file.metadata().map_err(|s| file_error(path, s))?.len();
    if length != artifact.bytes {
        return Err(length_error(artifact, length));
    }
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let read = file.read(&mut buffer).map_err(|s| file_error(path, s))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let actual = format!("{:x}", digest.finalize());
    if actual != artifact.sha256.as_str() {
        return Err(PackageError::Digest {
            expected: artifact.sha256.as_str().to_owned(),
            actual,
        });
    }
    Ok(())
}

fn length_error(artifact: &Artifact, actual: u64) -> PackageError {
    PackageError::Length {
        digest: artifact.sha256.as_str().to_owned(),
        expected: artifact.bytes,
        actual,
    }
}
