use std::fs::File;
use std::path::{Component, Path};

use super::io_error;
use crate::AviateSupervisorError;
use crate::document::AnchoredDirectory;

struct OwnedRoot {
    directory: File,
    identity: AnchoredDirectory,
}

pub(crate) fn create_artifact_root(
    root: &Path,
) -> Result<AnchoredDirectory, AviateSupervisorError> {
    validate_absent_root_path(root)?;
    create_private_directory(root)?;
    let owned = match bind_owned_root(root) {
        Ok(owned) => owned,
        Err(source) => return cleanup_unbound_root(root, source),
    };
    match finish_root_creation(&owned) {
        Ok(()) => Ok(owned.identity),
        Err(source) => cleanup_owned_root(&owned, source),
    }
}

pub(crate) fn inspect_directory(
    path: &Path,
    private: bool,
) -> Result<AnchoredDirectory, AviateSupervisorError> {
    if !is_normal_absolute(path) {
        return Err(AviateSupervisorError::invalid_request(
            "an anchored directory path is not normalized absolute",
        ));
    }
    let canonical = std::fs::canonicalize(path)
        .map_err(|source| io_error("canonicalize anchored directory", path, source))?;
    if canonical != path {
        return Err(AviateSupervisorError::invalid_request(
            "an anchored directory path is not canonical",
        ));
    }
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|source| io_error("inspect anchored directory", path, source))?;
    directory_identity(path, &metadata, private)
}

pub(crate) fn validate_directory(
    expected: &AnchoredDirectory,
    private: bool,
) -> Result<(), AviateSupervisorError> {
    let actual = inspect_directory(&expected.path, private)?;
    if actual != *expected {
        return Err(AviateSupervisorError::identity_mismatch(
            "an anchored directory identity changed",
        ));
    }
    Ok(())
}

fn validate_absent_root_path(root: &Path) -> Result<(), AviateSupervisorError> {
    if !is_normal_absolute(root) {
        return Err(AviateSupervisorError::invalid_request(
            "the launch-artifact root is not normalized absolute",
        ));
    }
    let parent = root.parent().ok_or_else(|| {
        AviateSupervisorError::invalid_request("the launch-artifact root has no parent")
    })?;
    let canonical_parent = std::fs::canonicalize(parent)
        .map_err(|source| io_error("canonicalize launch-artifact parent", parent, source))?;
    if canonical_parent != parent {
        return Err(AviateSupervisorError::invalid_request(
            "the launch-artifact parent is not canonical",
        ));
    }
    match std::fs::symlink_metadata(root) {
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(AviateSupervisorError::invalid_request(
            "the launch-artifact root already exists",
        )),
        Err(source) => Err(io_error("inspect launch-artifact root", root, source)),
    }
}

fn finish_root_creation(owned: &OwnedRoot) -> Result<(), AviateSupervisorError> {
    validate_owned_root(owned)?;
    owned
        .directory
        .sync_all()
        .map_err(|source| io_error("sync launch-artifact root", &owned.identity.path, source))?;
    sync_parent(&owned.identity.path)?;
    validate_owned_root(owned)
}

fn cleanup_owned_root<T>(
    owned: &OwnedRoot,
    source: AviateSupervisorError,
) -> Result<T, AviateSupervisorError> {
    let cleanup = validate_owned_root(owned)
        .and_then(|()| require_empty(&owned.identity.path))
        .and_then(|()| {
            std::fs::remove_dir(&owned.identity.path).map_err(|error| {
                io_error(
                    "remove partial launch-artifact root",
                    &owned.identity.path,
                    error,
                )
            })
        })
        .and_then(|()| sync_parent(&owned.identity.path));
    combine_cleanup(source, cleanup)
}

fn cleanup_unbound_root<T>(
    root: &Path,
    source: AviateSupervisorError,
) -> Result<T, AviateSupervisorError> {
    let cleanup = inspect_directory(root, true)
        .and_then(|_| require_empty(root))
        .and_then(|()| {
            std::fs::remove_dir(root)
                .map_err(|error| io_error("remove unbound launch-artifact root", root, error))
        })
        .and_then(|()| sync_parent(root));
    combine_cleanup(source, cleanup)
}

fn combine_cleanup<T>(
    source: AviateSupervisorError,
    cleanup: Result<(), AviateSupervisorError>,
) -> Result<T, AviateSupervisorError> {
    match cleanup {
        Ok(()) => Err(source),
        Err(cleanup) => Err(AviateSupervisorError::StartupCleanup {
            source: Box::new(source),
            cleanup: Box::new(cleanup),
        }),
    }
}

fn require_empty(root: &Path) -> Result<(), AviateSupervisorError> {
    let mut entries = std::fs::read_dir(root)
        .map_err(|source| io_error("scan launch-artifact root", root, source))?;
    if entries
        .next()
        .transpose()
        .map_err(|source| io_error("read launch-artifact root entry", root, source))?
        .is_some()
    {
        return Err(AviateSupervisorError::RecoveryBlocked {
            detail: "the launch-artifact root gained an unknown entry".to_owned(),
        });
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn create_private_directory(path: &Path) -> Result<(), AviateSupervisorError> {
    use std::os::unix::fs::DirBuilderExt as _;

    let mut builder = std::fs::DirBuilder::new();
    builder.mode(0o700);
    builder
        .create(path)
        .map_err(|source| io_error("create launch-artifact root", path, source))
}

fn bind_owned_root(path: &Path) -> Result<OwnedRoot, AviateSupervisorError> {
    let directory =
        File::open(path).map_err(|source| io_error("open launch-artifact root", path, source))?;
    let metadata = directory
        .metadata()
        .map_err(|source| io_error("inspect held launch-artifact root", path, source))?;
    let identity = directory_identity(path, &metadata, true)?;
    Ok(OwnedRoot {
        directory,
        identity,
    })
}

fn validate_owned_root(owned: &OwnedRoot) -> Result<(), AviateSupervisorError> {
    let held = owned.directory.metadata().map_err(|source| {
        io_error(
            "inspect held launch-artifact root",
            &owned.identity.path,
            source,
        )
    })?;
    let held = directory_identity(&owned.identity.path, &held, true)?;
    let named = inspect_directory(&owned.identity.path, true)?;
    if held != owned.identity || named != owned.identity {
        return Err(AviateSupervisorError::RecoveryBlocked {
            detail: "the owned launch-artifact root identity changed".to_owned(),
        });
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn directory_identity(
    path: &Path,
    metadata: &std::fs::Metadata,
    private: bool,
) -> Result<AnchoredDirectory, AviateSupervisorError> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let mode = metadata.permissions().mode() & 0o777;
    if metadata.file_type().is_symlink() || !metadata.is_dir() || (private && mode != 0o700) {
        return Err(AviateSupervisorError::identity_mismatch(
            "an anchored directory has invalid metadata",
        ));
    }
    Ok(AnchoredDirectory {
        path: path.to_path_buf(),
        device: metadata.dev(),
        inode: metadata.ino(),
        mode,
    })
}

fn is_normal_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
}

fn sync_parent(path: &Path) -> Result<(), AviateSupervisorError> {
    let parent = path.parent().ok_or_else(|| {
        AviateSupervisorError::invalid_request("the launch-artifact root has no parent")
    })?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error("sync launch-artifact parent", parent, source))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn create_private_directory(_path: &Path) -> Result<(), AviateSupervisorError> {
    Err(AviateSupervisorError::UnsupportedPlatform)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn directory_identity(
    _path: &Path,
    _metadata: &std::fs::Metadata,
    _private: bool,
) -> Result<AnchoredDirectory, AviateSupervisorError> {
    Err(AviateSupervisorError::UnsupportedPlatform)
}
