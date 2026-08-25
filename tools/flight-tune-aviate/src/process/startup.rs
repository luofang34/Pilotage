use std::io::Write as _;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

use super::SupervisedProcessRequest;
use crate::AviateSupervisorError;
use crate::document::ProcessIdentity;
use crate::protocol::encode_line;
use crate::supervisor::SupervisorBootstrap;

#[path = "startup/attestation.rs"]
mod attestation;
#[path = "startup/prepared.rs"]
mod prepared;
#[path = "startup/resources.rs"]
mod resources;

use resources::LaunchArtifacts;

pub(crate) use prepared::{
    PreparedLaunch, cancel_prepared, cancel_supported, prepare_supported, prepared_attestation,
    release_supported,
};

fn write_bootstrap(
    parent_lifetime: &mut ChildStdin,
    bootstrap: &SupervisorBootstrap,
) -> Result<(), AviateSupervisorError> {
    parent_lifetime
        .write_all(&encode_line(bootstrap)?)
        .and_then(|()| parent_lifetime.flush())
        .map_err(|source| process_io("write owner bootstrap", source))
}

fn spawn_owner(
    request: &SupervisedProcessRequest,
    artifacts: &LaunchArtifacts,
) -> Result<super::reaper::ReapableOwner, AviateSupervisorError> {
    let mut command = Command::new(&artifacts.supervisor.path);
    command
        .arg("supervise")
        .arg(&request.storage_root)
        .arg(&request.runtime_root)
        .env_clear()
        .current_dir(&artifacts.root.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    super::reaper::ReapableOwner::spawn(|| {
        command
            .spawn()
            .map_err(|source| process_io("spawn exact process owner", source))
    })
}

fn wait_parent_message<T: for<'de> serde::Deserialize<'de>>(
    listener: &std::os::unix::net::UnixListener,
    supervisor: &mut Child,
    timeout: Duration,
) -> Result<T, AviateSupervisorError> {
    let deadline = checked_deadline(timeout, "wait for owner readiness")?;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                return crate::protocol::read_message_until_blocking(stream, deadline);
            }
            Err(source) if source.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(source) => return Err(process_io("accept owner readiness", source)),
        }
        reject_exited_supervisor(supervisor)?;
        if Instant::now() >= deadline {
            return Err(AviateSupervisorError::Timeout {
                operation: "wait for owner readiness",
            });
        }
        std::thread::park_timeout(Duration::from_millis(1));
    }
}

fn reject_exited_supervisor(supervisor: &mut Child) -> Result<(), AviateSupervisorError> {
    if let Some(identity) = crate::inspection::inspect_lifetime(supervisor.id())?
        && identity.is_zombie
    {
        let status = supervisor
            .wait()
            .map_err(|source| process_io("reap failed process owner", source))?;
        return Err(AviateSupervisorError::SupervisorExited { status });
    }
    Ok(())
}

pub(super) fn verify_live(
    expected: &ProcessIdentity,
    process: &'static str,
) -> Result<(), AviateSupervisorError> {
    let Some(actual) =
        crate::inspection::inspect_process(expected.pid, expected.launch_argv_digest)?
    else {
        return Err(AviateSupervisorError::RecoveryBlocked {
            detail: format!("the exact {process} process is not running"),
        });
    };
    crate::inspection::validate_same_lifetime(expected, &actual, process)?;
    if actual != *expected {
        return Err(AviateSupervisorError::identity_mismatch(format!(
            "the exact {process} process identity changed"
        )));
    }
    let lifetime = crate::inspection::inspect_lifetime(expected.pid)?;
    if lifetime.is_none_or(|identity| identity.is_zombie) {
        return Err(AviateSupervisorError::RecoveryBlocked {
            detail: format!("the exact {process} process is not running"),
        });
    }
    Ok(())
}

pub(super) fn wait_for_supervisor_terminal(
    supervisor: &mut Child,
    identity: &ProcessIdentity,
    timeout: Duration,
) -> Result<(), AviateSupervisorError> {
    let deadline = checked_deadline(timeout, "wait for supervisor cleanup")?;
    loop {
        let actual = crate::inspection::inspect_lifetime(identity.pid)?;
        if terminal_owner_state(supervisor, identity, actual)? {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(AviateSupervisorError::Timeout {
                operation: "wait for supervisor cleanup",
            });
        }
        std::thread::park_timeout(Duration::from_millis(1));
    }
}

fn terminal_owner_state(
    supervisor: &mut Child,
    identity: &ProcessIdentity,
    actual: Option<crate::inspection::LifetimeIdentity>,
) -> Result<bool, AviateSupervisorError> {
    match actual {
        Some(actual)
            if actual.pid == identity.pid
                && actual.process_group == identity.process_group
                && actual.session_id == identity.session_id
                && actual.start == identity.start
                && actual.is_zombie =>
        {
            supervisor
                .wait()
                .map_err(|source| process_io("reap exact process owner", source))?;
            Ok(true)
        }
        Some(actual)
            if actual.pid == identity.pid
                && actual.process_group == identity.process_group
                && actual.session_id == identity.session_id
                && actual.start == identity.start =>
        {
            Ok(false)
        }
        Some(_) => Err(AviateSupervisorError::RecoveryBlocked {
            detail: "the owner process identifier names another lifetime".to_owned(),
        }),
        None => {
            supervisor
                .wait()
                .map_err(|source| process_io("reap exact process owner", source))?;
            Ok(true)
        }
    }
}

fn wait_for_unattested_supervisor(
    supervisor: &mut Child,
    timeout: Duration,
) -> Result<std::process::ExitStatus, AviateSupervisorError> {
    let deadline = checked_deadline(timeout, "clean failed process owner")?;
    loop {
        match crate::inspection::inspect_lifetime(supervisor.id())? {
            Some(identity) if !identity.is_zombie => {}
            Some(_) | None => {
                return supervisor
                    .wait()
                    .map_err(|source| process_io("reap failed process owner", source));
            }
        }
        if Instant::now() >= deadline {
            return Err(AviateSupervisorError::Timeout {
                operation: "clean failed process owner",
            });
        }
        std::thread::park_timeout(Duration::from_millis(1));
    }
}

fn validate_supervisor_cleanup_status(
    status: std::process::ExitStatus,
) -> Result<(), AviateSupervisorError> {
    if status.success() {
        Ok(())
    } else {
        Err(AviateSupervisorError::SupervisorExited { status })
    }
}

fn validate_request(request: &SupervisedProcessRequest) -> Result<(), AviateSupervisorError> {
    if request.supervisor_executable_digest.is_zero()
        || request.target_executable_digest.is_zero()
        || request.run_intent_digest.is_zero()
        || request.startup_timeout.is_zero()
        || request.cleanup_timeout.is_zero()
    {
        return Err(AviateSupervisorError::invalid_request(
            "the process supervision request is incomplete",
        ));
    }
    duration_millis(request.startup_timeout)?;
    duration_millis(request.cleanup_timeout)?;
    for path in [
        &request.supervisor_executable,
        &request.target_executable,
        &request.storage_root,
        &request.runtime_root,
        &request.artifact_root,
        &request.target_current_directory,
    ] {
        if !is_normal_absolute_path(path) {
            return Err(AviateSupervisorError::invalid_request(
                "a process supervision path is not normalized absolute UTF-8",
            ));
        }
    }
    require_canonical_existing(&request.supervisor_executable)?;
    require_canonical_existing(&request.target_executable)?;
    require_canonical_existing(&request.runtime_root)?;
    require_canonical_existing(&request.target_current_directory)?;
    validate_disjoint_roots(request)
}

fn is_normal_absolute_path(path: &Path) -> bool {
    let Some(text) = path.to_str() else {
        return false;
    };
    path.is_absolute()
        && !text
            .split(std::path::MAIN_SEPARATOR)
            .any(|component| matches!(component, "." | ".."))
        && path.components().all(|component| {
            matches!(
                component,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
}

fn validate_disjoint_roots(
    request: &SupervisedProcessRequest,
) -> Result<(), AviateSupervisorError> {
    let roots = [
        resolve_root(&request.storage_root)?,
        resolve_root(&request.runtime_root)?,
        resolve_root(&request.artifact_root)?,
        resolve_root(&request.target_current_directory)?,
    ];
    for (index, root) in roots.iter().enumerate() {
        for other in roots.iter().skip(index.wrapping_add(1)) {
            if root.starts_with(other) || other.starts_with(root) {
                return Err(AviateSupervisorError::invalid_request(
                    "a private root overlaps another root or the target current directory",
                ));
            }
        }
    }
    Ok(())
}

fn require_canonical_existing(path: &Path) -> Result<(), AviateSupervisorError> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|source| crate::inspection::io_error("canonicalize launch path", path, source))?;
    if canonical != path {
        return Err(AviateSupervisorError::invalid_request(
            "an existing launch path is not canonical",
        ));
    }
    Ok(())
}

fn resolve_root(path: &Path) -> Result<std::path::PathBuf, AviateSupervisorError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => {
            require_canonical_existing(path)?;
            Ok(path.to_path_buf())
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or_else(|| {
                AviateSupervisorError::invalid_request("an absent root has no parent")
            })?;
            require_canonical_existing(parent)?;
            Ok(path.to_path_buf())
        }
        Err(source) => Err(crate::inspection::io_error(
            "inspect launch root",
            path,
            source,
        )),
    }
}

fn cleanup_unbootstrapped<T>(
    listener: std::os::unix::net::UnixListener,
    listener_path: &Path,
    artifacts: &LaunchArtifacts,
    supervisor: &mut Child,
    timeout: Duration,
    source: AviateSupervisorError,
) -> Result<T, AviateSupervisorError> {
    let owner_cleanup = match wait_for_unattested_supervisor(supervisor, timeout) {
        Ok(status) => validate_supervisor_cleanup_status(status),
        Err(cleanup) => return combine_cleanup_results(source, Err(cleanup)),
    };
    let local_cleanup = cleanup_parent_resources(listener, listener_path, artifacts);
    combine_cleanup_results(source, merge_cleanup(owner_cleanup, local_cleanup))
}

fn cleanup_started_failure<T>(
    listener: std::os::unix::net::UnixListener,
    listener_path: &Path,
    artifacts: &LaunchArtifacts,
    supervisor: &mut Child,
    timeout: Duration,
    source: AviateSupervisorError,
) -> Result<T, AviateSupervisorError> {
    let owner_cleanup = match wait_for_unattested_supervisor(supervisor, timeout) {
        Ok(status) => validate_supervisor_cleanup_status(status),
        Err(cleanup) => return combine_cleanup_results(source, Err(cleanup)),
    };
    let local_cleanup = cleanup_parent_resources_after_owner(listener, listener_path, artifacts);
    combine_cleanup_results(source, merge_cleanup(owner_cleanup, local_cleanup))
}

fn cleanup_unstarted<T>(
    listener: std::os::unix::net::UnixListener,
    listener_path: &Path,
    artifacts: &LaunchArtifacts,
    source: AviateSupervisorError,
) -> Result<T, AviateSupervisorError> {
    combine_cleanup_results(
        source,
        cleanup_parent_resources(listener, listener_path, artifacts),
    )
}

fn cleanup_socket_only<T>(
    listener: std::os::unix::net::UnixListener,
    listener_path: &Path,
    source: AviateSupervisorError,
) -> Result<T, AviateSupervisorError> {
    drop(listener);
    combine_cleanup_results(
        source,
        crate::runtime_files::remove_exact_socket(listener_path),
    )
}

fn cleanup_parent_resources(
    listener: std::os::unix::net::UnixListener,
    listener_path: &Path,
    artifacts: &LaunchArtifacts,
) -> Result<(), AviateSupervisorError> {
    drop(listener);
    let socket = crate::runtime_files::remove_exact_socket(listener_path);
    let artifacts = resources::cleanup_launch_artifacts(artifacts);
    merge_cleanup(socket, artifacts)
}

fn cleanup_parent_resources_after_owner(
    listener: std::os::unix::net::UnixListener,
    listener_path: &Path,
    artifacts: &LaunchArtifacts,
) -> Result<(), AviateSupervisorError> {
    drop(listener);
    let socket = crate::runtime_files::remove_exact_socket(listener_path);
    let artifacts = resources::cleanup_launch_artifacts_after_owner(artifacts);
    merge_cleanup(socket, artifacts)
}

fn merge_cleanup(
    first: Result<(), AviateSupervisorError>,
    second: Result<(), AviateSupervisorError>,
) -> Result<(), AviateSupervisorError> {
    match (first, second) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(first), Err(second)) => Err(AviateSupervisorError::StartupCleanup {
            source: Box::new(first),
            cleanup: Box::new(second),
        }),
    }
}

fn combine_cleanup_results<T>(
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

pub(super) fn duration_millis(duration: Duration) -> Result<u64, AviateSupervisorError> {
    let milliseconds = u64::try_from(duration.as_millis()).map_err(|_| {
        AviateSupervisorError::invalid_request("a process timeout exceeds supported limits")
    })?;
    if milliseconds == 0 {
        return Err(AviateSupervisorError::invalid_request(
            "a process timeout is less than one millisecond",
        ));
    }
    Ok(milliseconds)
}

fn checked_deadline(
    timeout: Duration,
    operation: &'static str,
) -> Result<Instant, AviateSupervisorError> {
    Instant::now()
        .checked_add(timeout)
        .ok_or(AviateSupervisorError::Timeout { operation })
}

fn random_release_secret() -> Result<String, AviateSupervisorError> {
    use std::io::Read as _;

    let path = Path::new("/dev/urandom");
    let mut source = std::fs::File::open(path).map_err(|error| AviateSupervisorError::Io {
        operation: "open parent random source",
        path: path.to_path_buf(),
        source: error,
    })?;
    let mut bytes = [0_u8; 32];
    source
        .read_exact(&mut bytes)
        .map_err(|error| AviateSupervisorError::Io {
            operation: "read parent random source",
            path: path.to_path_buf(),
            source: error,
        })?;
    Ok(flight_tune::Digest::from_bytes(bytes).to_string())
}

fn process_io(operation: &'static str, source: std::io::Error) -> AviateSupervisorError {
    AviateSupervisorError::ProcessIo { operation, source }
}
