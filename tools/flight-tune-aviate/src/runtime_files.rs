use std::path::{Path, PathBuf};

use crate::AviateSupervisorError;

pub(crate) const PARENT_READY_SOCKET: &str = "parent-ready.sock";

pub(crate) fn validate_private_root(root: &Path) -> Result<(), AviateSupervisorError> {
    if !root.is_absolute() {
        return Err(AviateSupervisorError::invalid_request(
            "the supervisor runtime root is not absolute",
        ));
    }
    let metadata = std::fs::symlink_metadata(root)
        .map_err(|source| io_error("inspect supervisor runtime root", root, source))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AviateSupervisorError::invalid_request(
            "the supervisor runtime root is not one real directory",
        ));
    }
    let canonical = std::fs::canonicalize(root)
        .map_err(|source| io_error("canonicalize supervisor runtime root", root, source))?;
    if canonical != root {
        return Err(AviateSupervisorError::invalid_request(
            "the supervisor runtime root is not its canonical path",
        ));
    }
    validate_mode(root, &metadata)?;
    Ok(())
}

pub(crate) fn require_entries(root: &Path, expected: &[&str]) -> Result<(), AviateSupervisorError> {
    let mut entries = std::fs::read_dir(root)
        .map_err(|source| io_error("scan supervisor runtime root", root, source))?
        .map(|entry| {
            entry
                .map(|value| value.file_name())
                .map_err(|source| io_error("read supervisor runtime entry", root, source))
        })
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort();
    let mut expected = expected
        .iter()
        .map(std::ffi::OsString::from)
        .collect::<Vec<_>>();
    expected.sort();
    if entries != expected {
        return Err(AviateSupervisorError::invalid_request(
            "the supervisor runtime root contains residual entries",
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn bind_socket(
    path: &Path,
) -> Result<std::os::unix::net::UnixListener, AviateSupervisorError> {
    let listener = std::os::unix::net::UnixListener::bind(path)
        .map_err(|source| io_error("bind supervisor socket", path, source))?;
    if let Err(source) = listener.set_nonblocking(true) {
        drop(listener);
        let source = io_error("configure supervisor socket", path, source);
        return match remove_exact_socket(path) {
            Ok(()) => Err(source),
            Err(cleanup) => Err(AviateSupervisorError::StartupCleanup {
                source: Box::new(source),
                cleanup: Box::new(cleanup),
            }),
        };
    }
    Ok(listener)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) fn bind_socket(_path: &Path) -> Result<(), AviateSupervisorError> {
    Err(AviateSupervisorError::UnsupportedPlatform)
}

pub(crate) fn remove_exact_socket(path: &Path) -> Result<(), AviateSupervisorError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return sync_socket_parent(path);
        }
        Err(source) => return Err(io_error("inspect supervisor socket", path, source)),
    };
    if !is_socket(&metadata) {
        return Err(AviateSupervisorError::RecoveryBlocked {
            detail: format!("the runtime entry is not an exact socket: {path:?}"),
        });
    }
    std::fs::remove_file(path)
        .map_err(|source| io_error("remove supervisor socket", path, source))?;
    sync_socket_parent(path)
}

pub(crate) fn socket_path(root: &Path, name: &'static str) -> PathBuf {
    root.join(name)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn validate_mode(root: &Path, metadata: &std::fs::Metadata) -> Result<(), AviateSupervisorError> {
    use std::os::unix::fs::PermissionsExt as _;

    if metadata.permissions().mode() & 0o777 != 0o700 {
        return Err(AviateSupervisorError::invalid_request(format!(
            "the supervisor runtime root is not mode 0700: {root:?}"
        )));
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn validate_mode(_root: &Path, _metadata: &std::fs::Metadata) -> Result<(), AviateSupervisorError> {
    Err(AviateSupervisorError::UnsupportedPlatform)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn is_socket(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::FileTypeExt as _;

    metadata.file_type().is_socket()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn is_socket(_metadata: &std::fs::Metadata) -> bool {
    false
}

fn io_error(operation: &'static str, path: &Path, source: std::io::Error) -> AviateSupervisorError {
    AviateSupervisorError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

fn sync_socket_parent(path: &Path) -> Result<(), AviateSupervisorError> {
    let parent = path.parent().ok_or_else(|| {
        AviateSupervisorError::invalid_request("the supervisor socket has no parent")
    })?;
    std::fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error("sync supervisor runtime root", parent, source))
}
