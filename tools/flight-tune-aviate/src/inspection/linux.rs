use std::ffi::OsStr;
use std::fs::File;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::io::AsRawFd as _;
use std::path::{Path, PathBuf};

use crate::AviateSupervisorError;
use crate::document::{ProcessIdentity, ProcessStartIdentity};
use crate::inspection::{
    InspectionDeadline, LifetimeIdentity, ProcessGroupSnapshot, digest_argument_bytes,
    digest_open_file, digest_open_file_before, io_error, process_io,
};

const STABLE_SNAPSHOT_ATTEMPTS: usize = 3;

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExecutableObservation {
    path: PathBuf,
    digest: flight_tune::Digest,
    device: u64,
    inode: u64,
    bytes: u64,
}

trait ProcessSource {
    fn check_deadline(&mut self) -> Result<(), AviateSupervisorError>;
    fn lifetime(&mut self, pid: u32) -> Result<Option<LifetimeIdentity>, AviateSupervisorError>;
    fn command(&mut self, pid: u32) -> Result<Option<Vec<u8>>, AviateSupervisorError>;
    fn executable(
        &mut self,
        pid: u32,
    ) -> Result<Option<ExecutableObservation>, AviateSupervisorError>;
}

struct Procfs {
    deadline: Option<InspectionDeadline>,
}

enum StableSnapshot {
    Missing,
    Unstable {
        lifetime: LifetimeIdentity,
    },
    Stable {
        lifetime: LifetimeIdentity,
        command: Vec<u8>,
        executable: ExecutableObservation,
    },
}

pub(super) fn inspect_process(
    pid: u32,
    launch_argv_digest: flight_tune::Digest,
) -> Result<Option<ProcessIdentity>, AviateSupervisorError> {
    inspect_process_from(&mut Procfs { deadline: None }, pid, launch_argv_digest)
}

pub(super) fn inspect_process_before(
    pid: u32,
    launch_argv_digest: flight_tune::Digest,
    deadline: InspectionDeadline,
) -> Result<Option<ProcessIdentity>, AviateSupervisorError> {
    inspect_process_from(
        &mut Procfs {
            deadline: Some(deadline),
        },
        pid,
        launch_argv_digest,
    )
}

fn inspect_process_from(
    source: &mut impl ProcessSource,
    pid: u32,
    launch_argv_digest: flight_tune::Digest,
) -> Result<Option<ProcessIdentity>, AviateSupervisorError> {
    let mut lifetime_anchor = None;
    for _ in 0..STABLE_SNAPSHOT_ATTEMPTS {
        source.check_deadline()?;
        match read_stable_snapshot(source, pid)? {
            StableSnapshot::Missing => return Ok(None),
            StableSnapshot::Unstable { lifetime } => {
                bind_lifetime_anchor(&mut lifetime_anchor, &lifetime)?;
            }
            StableSnapshot::Stable {
                lifetime,
                command,
                executable,
            } => {
                bind_lifetime_anchor(&mut lifetime_anchor, &lifetime)?;
                return build_process_identity(
                    pid,
                    launch_argv_digest,
                    lifetime,
                    command,
                    executable,
                );
            }
        }
    }
    Err(AviateSupervisorError::identity_mismatch(
        "the Linux process image did not stabilize during inspection",
    ))
}

/// Loaded by path because this file is. The parent names it `platform`, and a
/// `#[path]` module's children are looked for inside a directory named for
/// that module — `platform/`, which nobody creates.
#[path = "linux/arguments.rs"]
mod arguments;

fn bind_lifetime_anchor(
    anchor: &mut Option<LifetimeIdentity>,
    actual: &LifetimeIdentity,
) -> Result<(), AviateSupervisorError> {
    match anchor {
        Some(expected) if expected != actual => Err(AviateSupervisorError::identity_mismatch(
            "the Linux process lifetime changed between inspection attempts",
        )),
        Some(_) => Ok(()),
        None => {
            *anchor = Some(actual.clone());
            Ok(())
        }
    }
}

fn build_process_identity(
    pid: u32,
    launch_argv_digest: flight_tune::Digest,
    lifetime: LifetimeIdentity,
    command: Vec<u8>,
    executable: ExecutableObservation,
) -> Result<Option<ProcessIdentity>, AviateSupervisorError> {
    let arguments = arguments::split(&command);
    let argv_digest = digest_argument_bytes(arguments.iter().copied());
    if argv_digest != launch_argv_digest {
        // Says WHICH arguments, because the reader is usually looking at a
        // failure they cannot reproduce, and the arguments usually name the
        // cause outright: a process caught mid-exec reports either its
        // parent's command line or none at all.
        return Err(AviateSupervisorError::identity_mismatch(format!(
            "the observed Linux arguments differ from the launch arguments: \
             pid {pid} reports {}",
            arguments::describe(&arguments),
        )));
    }
    Ok(Some(ProcessIdentity {
        pid,
        process_group: lifetime.process_group,
        session_id: lifetime.session_id,
        parent_pid: lifetime.parent_pid,
        real_user_id: lifetime.real_user_id,
        start: lifetime.start,
        executable: executable.path,
        executable_digest: executable.digest,
        launch_argv_digest,
        observed_argv_digest: Some(argv_digest),
    }))
}

fn read_stable_snapshot(
    source: &mut impl ProcessSource,
    pid: u32,
) -> Result<StableSnapshot, AviateSupervisorError> {
    source.check_deadline()?;
    let Some(before) = source.lifetime(pid)? else {
        return Ok(StableSnapshot::Missing);
    };
    let Some(first_command) = source.command(pid)? else {
        return classify_missing(source, pid, &before);
    };
    let Some(first_executable) = source.executable(pid)? else {
        return classify_missing(source, pid, &before);
    };
    let Some(final_command) = source.command(pid)? else {
        return classify_missing(source, pid, &before);
    };
    let Some(final_executable) = source.executable(pid)? else {
        return classify_missing(source, pid, &before);
    };
    let Some(after) = source.lifetime(pid)? else {
        return Ok(StableSnapshot::Missing);
    };
    source.check_deadline()?;
    if before != after {
        return Err(AviateSupervisorError::identity_mismatch(
            "the Linux process lifetime changed during inspection",
        ));
    }
    if first_command != final_command || first_executable != final_executable {
        return Ok(StableSnapshot::Unstable { lifetime: after });
    }
    Ok(StableSnapshot::Stable {
        lifetime: after,
        command: final_command,
        executable: final_executable,
    })
}

fn classify_missing(
    source: &mut impl ProcessSource,
    pid: u32,
    before: &LifetimeIdentity,
) -> Result<StableSnapshot, AviateSupervisorError> {
    match source.lifetime(pid)? {
        Some(after) if after == *before => Ok(StableSnapshot::Unstable { lifetime: after }),
        Some(_) => Err(AviateSupervisorError::identity_mismatch(
            "the Linux process lifetime changed during inspection",
        )),
        None => Ok(StableSnapshot::Missing),
    }
}

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

fn observe_executable(
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
fn observe_open_executable(
    file: File,
    live_path: &Path,
) -> Result<ExecutableObservation, AviateSupervisorError> {
    observe_open_executable_with_deadline(file, live_path, None)
}

fn observe_open_executable_with_deadline(
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

pub(super) fn inspect_lifetime(
    pid: u32,
) -> Result<Option<LifetimeIdentity>, AviateSupervisorError> {
    let stat_path = PathBuf::from(format!("/proc/{pid}/stat"));
    let Some(first_stat) = read_stat(&stat_path)? else {
        return Ok(None);
    };
    let (_, parent_pid, process_group, session_id, start_ticks) = parse_stat(&first_stat)?;
    let status_path = PathBuf::from(format!("/proc/{pid}/status"));
    let status = match std::fs::read_to_string(&status_path) {
        Ok(status) => status,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(io_error("inspect process owner", &status_path, source)),
    };
    let first_user_id = parse_real_user_id(&status)?;
    let Some(second_stat) = read_stat(&stat_path)? else {
        return Ok(None);
    };
    let (_, second_parent, second_group, second_session, second_start) = parse_stat(&second_stat)?;
    if (parent_pid, process_group, session_id, start_ticks)
        != (second_parent, second_group, second_session, second_start)
    {
        return Err(AviateSupervisorError::identity_mismatch(
            "the Linux process lifetime changed during inspection",
        ));
    }
    let final_status = match std::fs::read_to_string(&status_path) {
        Ok(status) => status,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(io_error("reinspect process owner", &status_path, source)),
    };
    let real_user_id = parse_real_user_id(&final_status)?;
    let Some(final_stat) = read_stat(&stat_path)? else {
        return Ok(None);
    };
    let (state, final_parent, final_group, final_session, final_start) = parse_stat(&final_stat)?;
    if first_user_id != real_user_id
        || (parent_pid, process_group, session_id, start_ticks)
            != (final_parent, final_group, final_session, final_start)
    {
        return Err(AviateSupervisorError::identity_mismatch(
            "the Linux process identity changed during inspection",
        ));
    }
    Ok(Some(lifetime(
        pid,
        parent_pid,
        process_group,
        session_id,
        real_user_id,
        start_ticks,
        state == "Z",
    )?))
}

fn read_stat(path: &Path) -> Result<Option<String>, AviateSupervisorError> {
    match std::fs::read_to_string(path) {
        Ok(stat) => Ok(Some(stat)),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(io_error("read process lifetime", path, source)),
    }
}

pub(super) fn process_group_snapshot(
    process_group: u32,
) -> Result<ProcessGroupSnapshot, AviateSupervisorError> {
    let entries = std::fs::read_dir("/proc")
        .map_err(|source| process_io("scan Linux process table", source))?;
    let mut members = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| process_io("read Linux process table", source))?;
        let bytes = entry.file_name();
        let Some(pid) = parse_pid(bytes.as_os_str()) else {
            continue;
        };
        if let Some(identity) = inspect_lifetime(pid)?
            && identity.process_group == process_group
        {
            members.push(identity);
        }
    }
    members.sort_by_key(|identity| identity.pid);
    let raw_pids = members.iter().map(|identity| identity.pid).collect();
    Ok(ProcessGroupSnapshot {
        raw_pids,
        observed: members,
        exited: Vec::new(),
        unclassified_pids: Vec::new(),
    })
}

fn parse_stat(stat: &str) -> Result<(&str, u32, u32, u32, u64), AviateSupervisorError> {
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

fn lifetime(
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

fn parse_pid(name: &OsStr) -> Option<u32> {
    let bytes = name.as_bytes();
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

fn parse_real_user_id(status: &str) -> Result<u32, AviateSupervisorError> {
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

#[cfg(test)]
#[path = "linux/tests.rs"]
mod tests;
