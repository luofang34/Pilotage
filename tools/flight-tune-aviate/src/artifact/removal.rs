use std::fs::File;

use super::io_error;
use super::root::validate_directory;
use super::staging::inspect_staged;
use crate::AviateSupervisorError;
use crate::document::{AnchoredDirectory, AnchoredExecutable};

pub(crate) fn remove_staged(
    root: &AnchoredDirectory,
    identity: &AnchoredExecutable,
) -> Result<(), AviateSupervisorError> {
    validate_directory(root, true)?;
    let actual = inspect_staged(&identity.path, identity.digest)?;
    if actual != *identity || identity.path.parent() != Some(root.path.as_path()) {
        return Err(AviateSupervisorError::RecoveryBlocked {
            detail: "the staged executable identity changed".to_owned(),
        });
    }
    std::fs::remove_file(&identity.path)
        .map_err(|source| io_error("remove staged executable", &identity.path, source))?;
    sync_directory(&root.path)?;
    validate_directory(root, true)
}

pub(crate) fn remove_artifact_root(root: &AnchoredDirectory) -> Result<(), AviateSupervisorError> {
    validate_directory(root, true)?;
    let mut entries = std::fs::read_dir(&root.path)
        .map_err(|source| io_error("scan launch-artifact root", &root.path, source))?;
    if entries
        .next()
        .transpose()
        .map_err(|source| io_error("read launch-artifact root", &root.path, source))?
        .is_some()
    {
        return Err(AviateSupervisorError::RecoveryBlocked {
            detail: "the launch-artifact root is not empty".to_owned(),
        });
    }
    validate_directory(root, true)?;
    std::fs::remove_dir(&root.path)
        .map_err(|source| io_error("remove launch-artifact root", &root.path, source))?;
    let parent = root.path.parent().ok_or_else(|| {
        AviateSupervisorError::invalid_request("the launch-artifact root has no parent")
    })?;
    sync_directory(parent)
}

pub(crate) fn stabilize_absent_artifact_root(
    root: &AnchoredDirectory,
) -> Result<(), AviateSupervisorError> {
    match std::fs::symlink_metadata(&root.path) {
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => {
            return Err(AviateSupervisorError::RecoveryBlocked {
                detail: "the launch-artifact root is still present".to_owned(),
            });
        }
        Err(source) => {
            return Err(io_error(
                "inspect absent launch-artifact root",
                &root.path,
                source,
            ));
        }
    }
    let parent = root.path.parent().ok_or_else(|| {
        AviateSupervisorError::invalid_request("the launch-artifact root has no parent")
    })?;
    sync_directory(parent)
}

fn sync_directory(path: &std::path::Path) -> Result<(), AviateSupervisorError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error("sync launch-artifact directory", path, source))
}
