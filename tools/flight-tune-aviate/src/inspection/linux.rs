use std::path::PathBuf;

use crate::AviateSupervisorError;
use crate::document::ProcessIdentity;
use crate::inspection::{
    InspectionDeadline, LifetimeIdentity, ProcessGroupSnapshot, digest_argument_bytes, io_error,
    process_io,
};

const STABLE_SNAPSHOT_ATTEMPTS: usize = 5;

/// Rest between unstable snapshots. Three instantaneous procfs reads
/// fit inside one execve, so an unpaced retry loop resolves nothing;
/// paced attempts span the window in which a process's command line is
/// legitimately in flight.
const SNAPSHOT_RETRY_PACE: std::time::Duration = std::time::Duration::from_millis(10);

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
                std::thread::sleep(SNAPSHOT_RETRY_PACE);
            }
            StableSnapshot::Stable {
                lifetime,
                command,
                executable,
            } => {
                bind_lifetime_anchor(&mut lifetime_anchor, &lifetime)?;
                // An EMPTY command line is not a different process — it
                // is the kernel's own statement that the image is in
                // flight (mid-exec) or gone (zombie), and both resolve
                // within the paced attempts or refuse as unstabilized.
                // Only a command line that says something DIFFERENT is a
                // mismatch.
                if arguments::split(&command).is_empty() {
                    std::thread::sleep(SNAPSHOT_RETRY_PACE);
                    continue;
                }
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
#[path = "linux/procfs.rs"]
mod procfs;
use procfs::{lifetime, parse_pid, parse_real_user_id, parse_stat, read_stat};

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

#[cfg(test)]
#[path = "linux/tests.rs"]
mod tests;
