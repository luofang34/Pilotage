use std::path::PathBuf;

use libproc::bsd_info::BSDInfo;
use libproc::pid_rusage::{RUsageInfoV2, pidrusage};
use libproc::proc_pid::{pidinfo, pidpath};
use libproc::processes::{ProcFilter, pids_by_type};
use sysctl::Sysctl as _;

use crate::AviateSupervisorError;
use crate::document::{ProcessIdentity, ProcessStartIdentity};
use crate::inspection::{
    ExitedGroupMember, LifetimeIdentity, ProcessGroupSnapshot, digest_file, process_io,
};

pub(super) fn inspect_process(
    pid: u32,
    launch_argv_digest: flight_tune::Digest,
) -> Result<Option<ProcessIdentity>, AviateSupervisorError> {
    let Some(lifetime) = inspect_lifetime(pid)? else {
        return Ok(None);
    };
    let pid_i32 = pid_value(pid)?;
    clear_errno();
    let executable = match pidpath(pid_i32) {
        Ok(path) => PathBuf::from(path),
        Err(detail) => {
            if inspect_lifetime(pid)?.is_none() {
                return Ok(None);
            }
            return Err(AviateSupervisorError::identity_mismatch(format!(
                "Darwin did not return the executable path: {detail}"
            )));
        }
    };
    let executable_digest = digest_file(&executable)?;
    let Some(final_lifetime) = inspect_lifetime(pid)? else {
        return Ok(None);
    };
    if final_lifetime != lifetime {
        return Err(AviateSupervisorError::identity_mismatch(
            "the Darwin process lifetime changed during inspection",
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
        observed_argv_digest: None,
    }))
}

pub(super) fn inspect_lifetime(
    pid: u32,
) -> Result<Option<LifetimeIdentity>, AviateSupervisorError> {
    let pid_i32 = pid_value(pid)?;
    let Some(first_session) = process_session(pid_i32)? else {
        return Ok(None);
    };
    let Some(first) = read_bsd_info(pid_i32, pid)? else {
        return Ok(None);
    };
    let Some(start_abstime) = process_start_abstime(pid_i32)? else {
        return Ok(None);
    };
    let Some(info) = read_bsd_info(pid_i32, pid)? else {
        return Ok(None);
    };
    if !same_bsd_lifetime(&first, &info) {
        return Err(AviateSupervisorError::identity_mismatch(
            "the Darwin process lifetime changed during lifetime inspection",
        ));
    }
    let Some(session_id) = process_session(pid_i32)? else {
        return Ok(None);
    };
    if session_id != first_session {
        return Err(AviateSupervisorError::identity_mismatch(
            "the Darwin process session changed during lifetime inspection",
        ));
    }
    let Some(final_start_abstime) = process_start_abstime(pid_i32)? else {
        return Ok(None);
    };
    if final_start_abstime != start_abstime {
        return Err(AviateSupervisorError::identity_mismatch(
            "the Darwin process start identity changed during inspection",
        ));
    }
    let microseconds = u32::try_from(info.pbi_start_tvusec).map_err(|_| {
        AviateSupervisorError::identity_mismatch("Darwin returned an invalid start microsecond")
    })?;
    Ok(Some(LifetimeIdentity {
        pid,
        process_group: info.pbi_pgid,
        session_id,
        parent_pid: info.pbi_ppid,
        real_user_id: info.pbi_ruid,
        start: ProcessStartIdentity::MacOs {
            boot_session_uuid: boot_session_uuid()?,
            seconds: info.pbi_start_tvsec,
            microseconds,
            start_abstime,
        },
        is_zombie: info.pbi_status == libc::SZOMB,
    }))
}

fn process_session(pid: i32) -> Result<Option<u32>, AviateSupervisorError> {
    let process = rustix::process::Pid::from_raw(pid).ok_or_else(|| {
        AviateSupervisorError::identity_mismatch("Darwin returned a zero process identifier")
    })?;
    match rustix::process::getsid(Some(process)) {
        Ok(session) => u32::try_from(session.as_raw_pid()).map(Some).map_err(|_| {
            AviateSupervisorError::identity_mismatch("Darwin returned an invalid process session")
        }),
        Err(rustix::io::Errno::SRCH) => Ok(None),
        Err(source) => Err(process_io(
            "inspect Darwin process session",
            std::io::Error::from_raw_os_error(source.raw_os_error()),
        )),
    }
}

pub(super) fn process_group_snapshot(
    process_group: u32,
) -> Result<ProcessGroupSnapshot, AviateSupervisorError> {
    let mut raw_pids = list_processes(ProcFilter::ByProgramGroup {
        pgrpid: process_group,
    })?;
    raw_pids.sort_unstable();
    let mut observed = Vec::new();
    let mut exited = Vec::new();
    let mut unclassified_pids = Vec::new();
    for pid in raw_pids.iter().copied() {
        if let Some(identity) = inspect_lifetime(pid)? {
            if identity.process_group != process_group {
                return Err(AviateSupervisorError::identity_mismatch(
                    "a Darwin process changed groups during inspection",
                ));
            }
            observed.push(identity);
        } else if let Some(member) = inspect_exited_group_member(pid)? {
            exited.push(member);
        } else {
            unclassified_pids.push(pid);
        }
    }
    observed.sort_by_key(|identity| identity.pid);
    exited.sort_by_key(|identity| identity.pid);
    Ok(ProcessGroupSnapshot {
        raw_pids,
        observed,
        exited,
        unclassified_pids,
    })
}

fn validate_bsd_info(pid: u32, info: &BSDInfo) -> Result<(), AviateSupervisorError> {
    if info.pbi_pid != pid || info.pbi_pgid == 0 || info.pbi_start_tvsec == 0 {
        return Err(AviateSupervisorError::identity_mismatch(
            "Darwin returned an incomplete process lifetime",
        ));
    }
    if info.pbi_start_tvusec >= 1_000_000 {
        return Err(AviateSupervisorError::identity_mismatch(
            "Darwin returned an out-of-range start microsecond",
        ));
    }
    Ok(())
}

fn read_bsd_info(pid: i32, expected: u32) -> Result<Option<BSDInfo>, AviateSupervisorError> {
    clear_errno();
    match pidinfo::<BSDInfo>(pid, 0) {
        Ok(info) => {
            validate_bsd_info(expected, &info)?;
            Ok(Some(info))
        }
        Err(_) if errno::errno().0 == libc::ESRCH => Ok(None),
        Err(detail) => Err(AviateSupervisorError::identity_mismatch(format!(
            "Darwin did not return the process lifetime: {detail}"
        ))),
    }
}

fn same_bsd_lifetime(first: &BSDInfo, second: &BSDInfo) -> bool {
    first.pbi_pid == second.pbi_pid
        && first.pbi_pgid == second.pbi_pgid
        && first.pbi_ppid == second.pbi_ppid
        && first.pbi_ruid == second.pbi_ruid
        && first.pbi_start_tvsec == second.pbi_start_tvsec
        && first.pbi_start_tvusec == second.pbi_start_tvusec
}

fn process_start_abstime(pid: i32) -> Result<Option<u64>, AviateSupervisorError> {
    clear_errno();
    match pidrusage::<RUsageInfoV2>(pid) {
        Ok(usage) if usage.ri_proc_start_abstime != 0 => Ok(Some(usage.ri_proc_start_abstime)),
        Ok(_) => Err(AviateSupervisorError::identity_mismatch(
            "Darwin returned an empty absolute process start time",
        )),
        Err(_) if errno::errno().0 == libc::ESRCH => Ok(None),
        Err(detail) => Err(AviateSupervisorError::identity_mismatch(format!(
            "Darwin did not return the absolute process start time: {detail}"
        ))),
    }
}

fn inspect_exited_group_member(
    pid: u32,
) -> Result<Option<ExitedGroupMember>, AviateSupervisorError> {
    clear_errno();
    match pidrusage::<RUsageInfoV2>(pid_value(pid)?) {
        Ok(usage) if usage.ri_proc_start_abstime != 0 && usage.ri_proc_exit_abstime != 0 => {
            Ok(Some(ExitedGroupMember {
                pid,
                start_abstime: usage.ri_proc_start_abstime,
            }))
        }
        Ok(_) => Err(AviateSupervisorError::RecoveryBlocked {
            detail: format!("Darwin cannot classify listed process-group member {pid}"),
        }),
        Err(_) if errno::errno().0 == libc::ESRCH => Ok(None),
        Err(detail) => Err(AviateSupervisorError::identity_mismatch(format!(
            "Darwin did not classify an exited process-group member: {detail}"
        ))),
    }
}

fn boot_session_uuid() -> Result<String, AviateSupervisorError> {
    let control = sysctl::Ctl::new("kern.bootsessionuuid").map_err(|source| {
        AviateSupervisorError::DarwinSystemControl {
            operation: "select boot-session identity",
            source,
        }
    })?;
    let value =
        control
            .value_string()
            .map_err(|source| AviateSupervisorError::DarwinSystemControl {
                operation: "read boot-session identity",
                source,
            })?;
    if !valid_uuid(&value) {
        return Err(AviateSupervisorError::identity_mismatch(
            "Darwin returned an invalid boot-session identity",
        ));
    }
    Ok(value.to_ascii_lowercase())
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
        && value.bytes().any(|byte| byte != b'0' && byte != b'-')
}

fn list_processes(filter: ProcFilter) -> Result<Vec<u32>, AviateSupervisorError> {
    clear_errno();
    pids_by_type(filter).map_err(|source| process_io("verify Darwin process existence", source))
}

fn clear_errno() {
    errno::set_errno(errno::Errno(0));
}

fn pid_value(pid: u32) -> Result<i32, AviateSupervisorError> {
    i32::try_from(pid).map_err(|_| {
        AviateSupervisorError::identity_mismatch("the process identifier exceeds Darwin limits")
    })
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::process::{Command, Stdio};

    use super::{inspect_lifetime, list_processes, process_group_snapshot, valid_uuid};
    use libproc::processes::ProcFilter;

    #[test]
    fn boot_session_uuid_has_exact_shape() {
        assert!(valid_uuid("01234567-89ab-cdef-0123-456789abcdef"));
        assert!(!valid_uuid("0123456789ab-cdef-0123-456789abcdef"));
        assert!(!valid_uuid("01234567-89ab-cdef-0123-456789abcdeg"));
        assert!(!valid_uuid("00000000-0000-0000-0000-000000000000"));
    }

    #[test]
    fn empty_group_ignores_stale_errno() {
        errno::set_errno(errno::Errno(libc::ESRCH));
        let pids = list_processes(ProcFilter::ByProgramGroup { pgrpid: u32::MAX })
            .expect("empty group scan");
        assert!(pids.is_empty());
    }

    #[test]
    fn ordinary_user_observes_self_and_synchronized_child() {
        let own = inspect_lifetime(std::process::id())
            .expect("inspect self")
            .expect("self exists");
        assert_ne!(
            own.start,
            crate::document::ProcessStartIdentity::MacOs {
                boot_session_uuid: String::new(),
                seconds: 0,
                microseconds: 0,
                start_abstime: 0,
            }
        );

        let mut child = Command::new("/bin/cat")
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn synchronized child");
        let observed = inspect_lifetime(child.id())
            .expect("inspect child")
            .expect("child exists");
        assert_eq!(observed.parent_pid, std::process::id());
        drop(child.stdin.take());
        child.wait().expect("reap synchronized child");
    }

    #[test]
    fn exited_unreaped_member_keeps_group_nonempty() {
        use std::os::unix::process::CommandExt as _;

        let mut child = Command::new("/usr/bin/true")
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .expect("spawn unreaped child");
        let raw_pid = i32::try_from(child.id()).expect("child PID fits POSIX");
        let pid = rustix::process::Pid::from_raw(raw_pid).expect("child PID is nonzero");
        let status = rustix::process::waitid(
            rustix::process::WaitId::Pid(pid),
            rustix::process::WaitIdOptions::EXITED | rustix::process::WaitIdOptions::NOWAIT,
        )
        .expect("wait for child exit without reaping")
        .expect("child exit status exists");
        assert_eq!(status.exit_status(), Some(0));

        let snapshot = process_group_snapshot(child.id()).expect("inspect unreaped child group");
        assert_eq!(snapshot.raw_pids, [child.id()]);
        assert!(snapshot.is_quiescent());
        assert!(!snapshot.is_empty());
        assert!(snapshot.unclassified_pids.is_empty());
        assert!(
            snapshot.observed.iter().any(|member| member.is_zombie)
                || snapshot
                    .exited
                    .iter()
                    .any(|member| member.pid == child.id())
        );

        let exit = child.wait().expect("reap synchronized child");
        assert!(exit.success());
        let empty = process_group_snapshot(child.id()).expect("inspect reaped child group");
        assert!(empty.is_empty());
    }
}
