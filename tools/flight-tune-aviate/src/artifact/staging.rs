use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::Path;

use sha2::{Digest as _, Sha256};

use super::io_error;
use super::root::validate_directory;
use crate::AviateSupervisorError;
use crate::document::{AnchoredDirectory, AnchoredExecutable};

const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;

struct OwnedPartial {
    file: File,
    path: std::path::PathBuf,
    device: u64,
    inode: u64,
}

pub(crate) fn stage_executable(
    root: &AnchoredDirectory,
    source_path: &Path,
    expected_digest: flight_tune::Digest,
    name: &'static str,
) -> Result<AnchoredExecutable, AviateSupervisorError> {
    stage(root, source_path, Some(expected_digest), name)
}

fn stage(
    root: &AnchoredDirectory,
    source_path: &Path,
    expected_digest: Option<flight_tune::Digest>,
    name: &'static str,
) -> Result<AnchoredExecutable, AviateSupervisorError> {
    validate_directory(root, true)?;
    let source_metadata = inspect_source(source_path)?;
    validate_source_size(source_metadata.len())?;
    let mut source = open_same_source(source_path, &source_metadata)?;
    let destination = root.path.join(name);
    let digest = copy_to_stage(&mut source, &source_metadata, expected_digest, &destination)?;
    validate_directory(root, true)?;
    sync_directory(&root.path)?;
    let identity = inspect_staged(&destination, digest)?;
    validate_directory(root, true)?;
    Ok(identity)
}

pub(crate) fn inspect_staged(
    path: &Path,
    expected_digest: flight_tune::Digest,
) -> Result<AnchoredExecutable, AviateSupervisorError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|source| io_error("inspect staged executable", path, source))?;
    validate_staged_metadata(path, &metadata)?;
    let digest = crate::inspection::digest_file(path)?;
    if digest != expected_digest {
        return Err(AviateSupervisorError::identity_mismatch(
            "the staged executable digest changed",
        ));
    }
    anchored_executable(path, digest, &metadata)
}

fn inspect_source(path: &Path) -> Result<std::fs::Metadata, AviateSupervisorError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|source| io_error("inspect launch source", path, source))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AviateSupervisorError::invalid_request(
            "a launch source is not one regular non-symlink file",
        ));
    }
    Ok(metadata)
}

fn open_same_source(
    path: &Path,
    expected: &std::fs::Metadata,
) -> Result<File, AviateSupervisorError> {
    let source = File::open(path).map_err(|error| io_error("open launch source", path, error))?;
    let actual = source
        .metadata()
        .map_err(|error| io_error("inspect opened launch source", path, error))?;
    validate_same_file(expected, &actual)?;
    Ok(source)
}

fn copy_to_stage(
    source: &mut File,
    source_metadata: &std::fs::Metadata,
    expected_digest: Option<flight_tune::Digest>,
    destination: &Path,
) -> Result<flight_tune::Digest, AviateSupervisorError> {
    let mut partial = create_destination(destination)?;
    let result = write_staged(source, source_metadata, expected_digest, &mut partial);
    match result {
        Ok(digest) => Ok(digest),
        Err(error) => cleanup_owned_partial(&partial, error),
    }
}

fn write_staged(
    source: &mut File,
    source_metadata: &std::fs::Metadata,
    expected_digest: Option<flight_tune::Digest>,
    partial: &mut OwnedPartial,
) -> Result<flight_tune::Digest, AviateSupervisorError> {
    let (digest, bytes) = copy_and_digest(source, &mut partial.file, &partial.path)?;
    validate_source_end(source, source_metadata, bytes, &partial.path)?;
    if expected_digest.is_some_and(|expected| expected != digest) {
        return Err(AviateSupervisorError::identity_mismatch(
            "the staged executable bytes differ from the requested digest",
        ));
    }
    set_executable_mode(&partial.file, &partial.path)?;
    partial
        .file
        .sync_all()
        .map_err(|error| io_error("sync staged executable", &partial.path, error))?;
    Ok(digest)
}

fn copy_and_digest(
    source: &mut File,
    destination: &mut File,
    path: &Path,
) -> Result<(flight_tune::Digest, u64), AviateSupervisorError> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let count = source
            .read(&mut buffer)
            .map_err(|error| io_error("read launch source", path, error))?;
        if count == 0 {
            break;
        }
        total = add_count(total, count)?;
        hasher.update(&buffer[..count]);
        destination
            .write_all(&buffer[..count])
            .map_err(|error| io_error("write staged executable", path, error))?;
    }
    Ok((
        flight_tune::Digest::from_bytes(hasher.finalize().into()),
        total,
    ))
}

fn add_count(total: u64, count: usize) -> Result<u64, AviateSupervisorError> {
    let count = u64::try_from(count).map_err(|_| {
        AviateSupervisorError::invalid_request("an executable read count exceeds limits")
    })?;
    let total = total.checked_add(count).ok_or_else(|| {
        AviateSupervisorError::invalid_request("an executable byte count overflowed")
    })?;
    validate_source_size(total)?;
    Ok(total)
}

fn validate_source_size(bytes: u64) -> Result<(), AviateSupervisorError> {
    if bytes == 0 || bytes > MAX_ARTIFACT_BYTES {
        return Err(AviateSupervisorError::invalid_request(
            "a launch source exceeds the executable byte limits",
        ));
    }
    Ok(())
}

fn validate_source_end(
    source: &File,
    initial: &std::fs::Metadata,
    bytes: u64,
    path: &Path,
) -> Result<(), AviateSupervisorError> {
    let final_metadata = source
        .metadata()
        .map_err(|error| io_error("inspect copied launch source", path, error))?;
    validate_same_file(initial, &final_metadata)?;
    if final_metadata.len() != bytes {
        return Err(AviateSupervisorError::identity_mismatch(
            "the launch source size changed during staging",
        ));
    }
    Ok(())
}

fn cleanup_owned_partial<T>(
    partial: &OwnedPartial,
    source: AviateSupervisorError,
) -> Result<T, AviateSupervisorError> {
    let cleanup = validate_owned_partial(partial)
        .and_then(|()| {
            std::fs::remove_file(&partial.path)
                .map_err(|error| io_error("remove partial staged executable", &partial.path, error))
        })
        .and_then(|()| sync_parent(&partial.path));
    match cleanup {
        Ok(()) => Err(source),
        Err(cleanup) => Err(AviateSupervisorError::StartupCleanup {
            source: Box::new(source),
            cleanup: Box::new(cleanup),
        }),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn create_destination(path: &Path) -> Result<OwnedPartial, AviateSupervisorError> {
    use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};

    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o500)
        .open(path)
        .map_err(|source| io_error("create staged executable", path, source))?;
    let metadata = file
        .metadata()
        .map_err(|source| io_error("inspect partial staged executable", path, source))?;
    Ok(OwnedPartial {
        file,
        path: path.to_path_buf(),
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn set_executable_mode(file: &File, path: &Path) -> Result<(), AviateSupervisorError> {
    use std::os::unix::fs::PermissionsExt as _;

    file.set_permissions(std::fs::Permissions::from_mode(0o500))
        .map_err(|source| io_error("set staged executable mode", path, source))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn validate_owned_partial(partial: &OwnedPartial) -> Result<(), AviateSupervisorError> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let metadata = std::fs::symlink_metadata(&partial.path)
        .map_err(|source| io_error("inspect partial staged executable", &partial.path, source))?;
    let held = partial
        .file
        .metadata()
        .map_err(|source| io_error("inspect held staged executable", &partial.path, source))?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.dev() != partial.device
        || metadata.ino() != partial.inode
        || held.dev() != partial.device
        || held.ino() != partial.inode
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o777 != 0o500
    {
        return Err(AviateSupervisorError::RecoveryBlocked {
            detail: "the owned partial executable identity changed".to_owned(),
        });
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn validate_same_file(
    first: &std::fs::Metadata,
    second: &std::fs::Metadata,
) -> Result<(), AviateSupervisorError> {
    use std::os::unix::fs::MetadataExt as _;

    if first.dev() != second.dev() || first.ino() != second.ino() {
        return Err(AviateSupervisorError::identity_mismatch(
            "the launch source changed while it was opened",
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn validate_staged_metadata(
    path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<(), AviateSupervisorError> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o777 != 0o500
    {
        return Err(AviateSupervisorError::identity_mismatch(format!(
            "the staged executable metadata is invalid: {path:?}"
        )));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn anchored_executable(
    path: &Path,
    digest: flight_tune::Digest,
    metadata: &std::fs::Metadata,
) -> Result<AnchoredExecutable, AviateSupervisorError> {
    use std::os::unix::fs::MetadataExt as _;

    Ok(AnchoredExecutable {
        path: path.to_path_buf(),
        digest,
        device: metadata.dev(),
        inode: metadata.ino(),
        bytes: metadata.len(),
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn create_destination(_path: &Path) -> Result<OwnedPartial, AviateSupervisorError> {
    Err(AviateSupervisorError::UnsupportedPlatform)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn set_executable_mode(_file: &File, _path: &Path) -> Result<(), AviateSupervisorError> {
    Err(AviateSupervisorError::UnsupportedPlatform)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn validate_owned_partial(_partial: &OwnedPartial) -> Result<(), AviateSupervisorError> {
    Err(AviateSupervisorError::UnsupportedPlatform)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn validate_same_file(
    _first: &std::fs::Metadata,
    _second: &std::fs::Metadata,
) -> Result<(), AviateSupervisorError> {
    Err(AviateSupervisorError::UnsupportedPlatform)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn validate_staged_metadata(
    _path: &Path,
    _metadata: &std::fs::Metadata,
) -> Result<(), AviateSupervisorError> {
    Err(AviateSupervisorError::UnsupportedPlatform)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn anchored_executable(
    _path: &Path,
    _digest: flight_tune::Digest,
    _metadata: &std::fs::Metadata,
) -> Result<AnchoredExecutable, AviateSupervisorError> {
    Err(AviateSupervisorError::UnsupportedPlatform)
}

fn sync_directory(path: &Path) -> Result<(), AviateSupervisorError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error("sync launch-artifact directory", path, source))
}

fn sync_parent(path: &Path) -> Result<(), AviateSupervisorError> {
    let parent = path
        .parent()
        .ok_or_else(|| AviateSupervisorError::invalid_request("an artifact path has no parent"))?;
    sync_directory(parent)
}
