use std::fs::File;
use std::io::Read as _;
use std::path::Path;
use std::time::Instant;

use sha2::{Digest as _, Sha256};

use crate::AviateSupervisorError;
use crate::document::{BootIdentity, ProcessIdentity, ProcessStartIdentity};

#[cfg(target_os = "linux")]
#[path = "inspection/linux.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "inspection/macos.rs"]
mod platform;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LifetimeIdentity {
    pub(crate) pid: u32,
    pub(crate) process_group: u32,
    pub(crate) session_id: u32,
    pub(crate) parent_pid: u32,
    pub(crate) real_user_id: u32,
    pub(crate) start: ProcessStartIdentity,
    pub(crate) is_zombie: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExitedGroupMember {
    pub(crate) pid: u32,
    pub(crate) start_abstime: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProcessGroupSnapshot {
    pub(crate) raw_pids: Vec<u32>,
    pub(crate) observed: Vec<LifetimeIdentity>,
    pub(crate) exited: Vec<ExitedGroupMember>,
    pub(crate) unclassified_pids: Vec<u32>,
}

#[derive(Clone, Copy)]
pub(crate) struct InspectionDeadline {
    deadline: Instant,
    operation: &'static str,
}

impl InspectionDeadline {
    pub(crate) const fn new(deadline: Instant, operation: &'static str) -> Self {
        Self {
            deadline,
            operation,
        }
    }

    pub(crate) fn check(self) -> Result<(), AviateSupervisorError> {
        if Instant::now() >= self.deadline {
            Err(AviateSupervisorError::Timeout {
                operation: self.operation,
            })
        } else {
            Ok(())
        }
    }
}

impl ProcessGroupSnapshot {
    pub(crate) fn is_empty(&self) -> bool {
        self.raw_pids.is_empty()
    }

    pub(crate) fn is_quiescent(&self) -> bool {
        self.unclassified_pids.is_empty()
            && self.observed.iter().all(|member| member.is_zombie)
            && self.raw_pids.len() == self.observed.len().wrapping_add(self.exited.len())
    }
}

pub(crate) fn inspect_process(
    pid: u32,
    launch_argv_digest: flight_tune::Digest,
) -> Result<Option<ProcessIdentity>, AviateSupervisorError> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        platform::inspect_process(pid, launch_argv_digest)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (pid, launch_argv_digest);
        Err(AviateSupervisorError::UnsupportedPlatform)
    }
}

pub(crate) fn inspect_process_before(
    pid: u32,
    launch_argv_digest: flight_tune::Digest,
    deadline: Instant,
    operation: &'static str,
) -> Result<Option<ProcessIdentity>, AviateSupervisorError> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        platform::inspect_process_before(
            pid,
            launch_argv_digest,
            InspectionDeadline::new(deadline, operation),
        )
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (pid, launch_argv_digest, deadline, operation);
        Err(AviateSupervisorError::UnsupportedPlatform)
    }
}

pub(crate) fn inspect_lifetime(
    pid: u32,
) -> Result<Option<LifetimeIdentity>, AviateSupervisorError> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        platform::inspect_lifetime(pid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        Err(AviateSupervisorError::UnsupportedPlatform)
    }
}

pub(crate) fn process_group_snapshot(
    process_group: u32,
) -> Result<ProcessGroupSnapshot, AviateSupervisorError> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        platform::process_group_snapshot(process_group)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = process_group;
        Err(AviateSupervisorError::UnsupportedPlatform)
    }
}

pub(crate) fn process_group_is_absent(process_group: u32) -> Result<bool, AviateSupervisorError> {
    let raw = i32::try_from(process_group).map_err(|_| {
        AviateSupervisorError::identity_mismatch("the process group exceeds POSIX limits")
    })?;
    let group = rustix::process::Pid::from_raw(raw)
        .ok_or_else(|| AviateSupervisorError::identity_mismatch("the process group is zero"))?;
    match rustix::process::test_kill_process_group(group) {
        Ok(()) => Ok(false),
        Err(rustix::io::Errno::SRCH) => Ok(true),
        Err(source) => Err(process_io(
            "test exact process-group absence",
            std::io::Error::from_raw_os_error(source.raw_os_error()),
        )),
    }
}

pub(crate) fn digest_file(path: &Path) -> Result<flight_tune::Digest, AviateSupervisorError> {
    let mut file =
        File::open(path).map_err(|source| io_error("open executable for hashing", path, source))?;
    digest_open_file(&mut file).map_err(|source| io_error("hash executable", path, source))
}

pub(crate) fn digest_arguments(arguments: &[String]) -> flight_tune::Digest {
    digest_argument_bytes(arguments.iter().map(String::as_bytes))
}

pub(crate) fn validate_same_lifetime(
    expected: &ProcessIdentity,
    actual: &ProcessIdentity,
    process: &'static str,
) -> Result<(), AviateSupervisorError> {
    if expected.pid != actual.pid
        || expected.process_group != actual.process_group
        || expected.session_id != actual.session_id
        || expected.parent_pid != actual.parent_pid
        || expected.real_user_id != actual.real_user_id
        || expected.start != actual.start
    {
        return Err(AviateSupervisorError::identity_mismatch(format!(
            "the {process} process lifetime changed"
        )));
    }
    Ok(())
}

pub(crate) fn validate_absent_or_exact(
    expected: &ProcessIdentity,
    actual: Option<LifetimeIdentity>,
    process: &'static str,
) -> Result<bool, AviateSupervisorError> {
    match actual {
        None => Ok(true),
        Some(actual)
            if actual.pid == expected.pid
                && actual.process_group == expected.process_group
                && actual.session_id == expected.session_id
                && actual.start == expected.start =>
        {
            Ok(false)
        }
        Some(_) => Err(AviateSupervisorError::RecoveryBlocked {
            detail: format!("the {process} process identifier names another lifetime"),
        }),
    }
}

pub(crate) fn current_boot_identity() -> Result<BootIdentity, AviateSupervisorError> {
    let current = inspect_lifetime(std::process::id())?.ok_or_else(|| {
        AviateSupervisorError::identity_mismatch("the current process identity is unavailable")
    })?;
    Ok(current.start.boot_identity())
}

pub(crate) fn process_io(operation: &'static str, source: std::io::Error) -> AviateSupervisorError {
    AviateSupervisorError::ProcessIo { operation, source }
}

pub(crate) fn io_error(
    operation: &'static str,
    path: &Path,
    source: std::io::Error,
) -> AviateSupervisorError {
    AviateSupervisorError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

pub(crate) fn digest_argument_bytes<'a>(
    arguments: impl IntoIterator<Item = &'a [u8]>,
) -> flight_tune::Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"flight-tune-aviate-argv-v1\0");
    for argument in arguments {
        hasher.update((argument.len() as u64).to_be_bytes());
        hasher.update(argument);
    }
    flight_tune::Digest::from_bytes(hasher.finalize().into())
}

pub(crate) fn digest_open_file(reader: &mut File) -> std::io::Result<flight_tune::Digest> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(flight_tune::Digest::from_bytes(hasher.finalize().into()))
}

pub(crate) fn digest_open_file_before(
    reader: &mut File,
    path: &Path,
    deadline: InspectionDeadline,
) -> Result<flight_tune::Digest, AviateSupervisorError> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        deadline.check()?;
        let count = reader
            .read(&mut buffer)
            .map_err(|source| io_error("hash executable before deadline", path, source))?;
        if count == 0 {
            deadline.check()?;
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(flight_tune::Digest::from_bytes(hasher.finalize().into()))
}

#[cfg(target_os = "macos")]
pub(crate) fn digest_file_before(
    path: &Path,
    deadline: InspectionDeadline,
) -> Result<flight_tune::Digest, AviateSupervisorError> {
    deadline.check()?;
    let mut file = File::open(path)
        .map_err(|source| io_error("open executable before deadline", path, source))?;
    digest_open_file_before(&mut file, path, deadline)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests;
