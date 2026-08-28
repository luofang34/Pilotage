//! The procfs half of the Linux inspection: reading and parsing what
//! the kernel publishes about a process. The inspection LOGIC — what
//! counts as stable, matching, or in flight — lives in the parent.

use std::ffi::OsStr;
use std::fs::File;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::io::AsRawFd as _;
use std::path::{Path, PathBuf};

use super::{
    ExecutableObservation, InspectionDeadline, LifetimeIdentity, ProcessSource, Procfs,
    inspect_lifetime,
};
use crate::AviateSupervisorError;
use crate::document::ProcessStartIdentity;
use crate::inspection::{digest_open_file, digest_open_file_before, io_error};

impl ProcessSource for Procfs {
    fn check_deadline(&mut self) -> Result<(), AviateSupervisorError> {
        self.deadline.map_or(Ok(()), InspectionDeadline::check)
    }

    fn lifetime(&mut self, pid: u32) -> Result<Option<LifetimeIdentity>, AviateSupervisorError> {
        self.check_deadline()?;
        let lifetime = inspect_lifetime(pid)?;
        self.check_deadline()?;
        Ok(lifetime)
    }

    fn command(&mut self, pid: u32) -> Result<Option<Vec<u8>>, AviateSupervisorError> {
        self.check_deadline()?;
        let path = PathBuf::from(format!("/proc/{pid}/cmdline"));
        let command = match std::fs::read(&path) {
            Ok(command) => Ok(Some(command)),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(io_error("read live argument vector", &path, source)),
        }?;
        self.check_deadline()?;
        Ok(command)
    }

    fn executable(
        &mut self,
        pid: u32,
    ) -> Result<Option<ExecutableObservation>, AviateSupervisorError> {
        self.check_deadline()?;
        let executable = observe_executable(pid, self.deadline)?;
        self.check_deadline()?;
        Ok(executable)
    }
}

pub(super) fn observe_executable(
    pid: u32,
    deadline: Option<InspectionDeadline>,
) -> Result<Option<ExecutableObservation>, AviateSupervisorError> {
    let live_path = PathBuf::from(format!("/proc/{pid}/exe"));
    if let Some(deadline) = deadline {
        deadline.check()?;
    }
    let file = match File::open(&live_path) {
        Ok(file) => file,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(io_error("open live executable", &live_path, source)),
    };
    observe_open_executable_with_deadline(file, &live_path, deadline).map(Some)
}

#[cfg(test)]
pub(super) fn observe_open_executable(
    file: File,
    live_path: &Path,
) -> Result<ExecutableObservation, AviateSupervisorError> {
    observe_open_executable_with_deadline(file, live_path, None)
}

pub(super) fn observe_open_executable_with_deadline(
    mut file: File,
    live_path: &Path,
    deadline: Option<InspectionDeadline>,
) -> Result<ExecutableObservation, AviateSupervisorError> {
    let fd_path = PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()));
    let path = std::fs::read_link(&fd_path)
        .map_err(|source| io_error("read held executable link", &fd_path, source))?;
    let metadata = file
        .metadata()
        .map_err(|source| io_error("inspect held executable", live_path, source))?;
    let digest = match deadline {
        Some(deadline) => digest_open_file_before(&mut file, live_path, deadline)?,
        None => digest_open_file(&mut file)
            .map_err(|source| io_error("hash held executable", live_path, source))?,
    };
    Ok(ExecutableObservation {
        path,
        digest,
        device: metadata.dev(),
        inode: metadata.ino(),
        bytes: metadata.len(),
    })
}

pub(super) fn read_stat(path: &Path) -> Result<Option<String>, AviateSupervisorError> {
    match std::fs::read_to_string(path) {
        Ok(stat) => Ok(Some(stat)),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(io_error("read process lifetime", path, source)),
    }
}

pub(super) fn parse_stat(stat: &str) -> Result<(&str, u32, u32, u32, u64), AviateSupervisorError> {
    let close = stat.rfind(')').ok_or_else(|| {
        AviateSupervisorError::identity_mismatch("the Linux process stat has no command boundary")
    })?;
    let fields = stat
        .get(close.saturating_add(2)..)
        .ok_or_else(|| {
            AviateSupervisorError::identity_mismatch("the Linux process stat is truncated")
        })?
        .split_whitespace()
        .collect::<Vec<_>>();
    if fields.len() <= 19 {
        return Err(AviateSupervisorError::identity_mismatch(
            "the Linux process stat has too few fields",
        ));
    }
    let process_group = fields[2].parse::<u32>().map_err(|_| {
        AviateSupervisorError::identity_mismatch("the Linux process group is invalid")
    })?;
    let parent_pid = fields[1].parse::<u32>().map_err(|_| {
        AviateSupervisorError::identity_mismatch("the Linux parent process is invalid")
    })?;
    let session_id = fields[3].parse::<u32>().map_err(|_| {
        AviateSupervisorError::identity_mismatch("the Linux process session is invalid")
    })?;
    let start_ticks = fields[19].parse::<u64>().map_err(|_| {
        AviateSupervisorError::identity_mismatch("the Linux process start tick is invalid")
    })?;
    Ok((
        fields[0],
        parent_pid,
        process_group,
        session_id,
        start_ticks,
    ))
}

pub(super) fn lifetime(
    pid: u32,
    parent_pid: u32,
    process_group: u32,
    session_id: u32,
    real_user_id: u32,
    start_ticks: u64,
    is_zombie: bool,
) -> Result<LifetimeIdentity, AviateSupervisorError> {
    let boot_path = Path::new("/proc/sys/kernel/random/boot_id");
    let boot_id = std::fs::read_to_string(boot_path)
        .map_err(|source| io_error("read Linux boot identity", boot_path, source))?
        .trim()
        .to_owned();
    if boot_id.is_empty() || start_ticks == 0 {
        return Err(AviateSupervisorError::identity_mismatch(
            "the Linux process start identity is incomplete",
        ));
    }
    let start = ProcessStartIdentity::Linux {
        boot_id,
        start_ticks,
    };
    if !start.boot_identity().is_valid() {
        return Err(AviateSupervisorError::identity_mismatch(
            "the Linux boot identity is invalid",
        ));
    }
    Ok(LifetimeIdentity {
        pid,
        process_group,
        session_id,
        parent_pid,
        real_user_id,
        start,
        is_zombie,
    })
}

pub(super) fn parse_pid(name: &OsStr) -> Option<u32> {
    let bytes = name.as_bytes();
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

pub(super) fn parse_real_user_id(status: &str) -> Result<u32, AviateSupervisorError> {
    let value = status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|line| line.split_whitespace().next())
        .ok_or_else(|| {
            AviateSupervisorError::identity_mismatch(
                "the Linux process status has no real user identity",
            )
        })?;
    value.parse().map_err(|_| {
        AviateSupervisorError::identity_mismatch(
            "the Linux process status has an invalid real user identity",
        )
    })
}
