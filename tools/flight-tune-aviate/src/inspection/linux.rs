use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};

use crate::AviateSupervisorError;
use crate::document::{ProcessIdentity, ProcessStartIdentity};
use crate::inspection::{
    LifetimeIdentity, ProcessGroupSnapshot, digest_argument_bytes, digest_file, io_error,
    process_io,
};

pub(super) fn inspect_process(
    pid: u32,
    launch_argv_digest: flight_tune::Digest,
) -> Result<Option<ProcessIdentity>, AviateSupervisorError> {
    let Some(lifetime) = inspect_lifetime(pid)? else {
        return Ok(None);
    };
    let executable_link = PathBuf::from(format!("/proc/{pid}/exe"));
    let executable = match std::fs::read_link(&executable_link) {
        Ok(path) => path,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(io_error(
                "read live executable link",
                &executable_link,
                source,
            ));
        }
    };
    let executable_digest = digest_file(&executable_link)?;
    let command_path = PathBuf::from(format!("/proc/{pid}/cmdline"));
    let command = std::fs::read(&command_path)
        .map_err(|source| io_error("read live argument vector", &command_path, source))?;
    let mut arguments = command.split(|byte| *byte == 0).collect::<Vec<_>>();
    if arguments.last().is_some_and(|argument| argument.is_empty()) {
        arguments.pop();
    }
    let argv_digest = digest_argument_bytes(arguments);
    if argv_digest != launch_argv_digest {
        return Err(AviateSupervisorError::identity_mismatch(
            "the observed Linux arguments differ from the launch arguments",
        ));
    }
    let Some(final_lifetime) = inspect_lifetime(pid)? else {
        return Ok(None);
    };
    if final_lifetime != lifetime {
        return Err(AviateSupervisorError::identity_mismatch(
            "the Linux process lifetime changed during inspection",
        ));
    }
    Ok(Some(ProcessIdentity {
        pid,
        process_group: lifetime.process_group,
        session_id: lifetime.session_id,
        parent_pid: lifetime.parent_pid,
        real_user_id: lifetime.real_user_id,
        start: lifetime.start,
        executable,
        executable_digest,
        launch_argv_digest,
        observed_argv_digest: Some(argv_digest),
    }))
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
#[allow(clippy::expect_used)]
mod tests {
    use super::parse_stat;

    #[test]
    fn stat_parser_uses_the_last_command_boundary() {
        let mut fields = vec!["S", "42", "43", "44"];
        fields.extend(std::iter::repeat_n("0", 15));
        fields.push("99");
        let stat = format!("77 (name) with ) marks) {}", fields.join(" "));

        let parsed = parse_stat(&stat).expect("valid stat");

        assert_eq!(parsed, ("S", 42, 43, 44, 99));
    }
}
