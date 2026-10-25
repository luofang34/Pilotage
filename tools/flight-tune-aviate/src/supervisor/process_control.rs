use std::process::Child;

use crate::AviateSupervisorError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProcessGroupSignal {
    Delivered,
    GroupMissing,
    /// The kernel refused the signal. macOS refuses a group whose only members
    /// are unreaped zombies, so the caller decides from a group snapshot.
    Refused,
}

/// Sends `SIGKILL` to `process_group` and accepts an undelivered signal only
/// when a fresh snapshot shows that no member of the group still runs.
pub(super) fn stop_process_group(process_group: u32) -> Result<(), AviateSupervisorError> {
    match signal_process_group(process_group)? {
        ProcessGroupSignal::Delivered => Ok(()),
        ProcessGroupSignal::GroupMissing | ProcessGroupSignal::Refused
            if crate::inspection::process_group_snapshot(process_group)?.is_quiescent() =>
        {
            Ok(())
        }
        ProcessGroupSignal::GroupMissing => Err(AviateSupervisorError::RecoveryBlocked {
            detail: "the target process group disappeared before it became quiescent".to_owned(),
        }),
        ProcessGroupSignal::Refused => Err(AviateSupervisorError::RecoveryBlocked {
            detail: "the target process group refused the stop signal while a member still runs"
                .to_owned(),
        }),
    }
}

pub(super) fn stop_child(child: &mut Child) -> std::io::Result<()> {
    child.kill()
}

pub(super) fn signal_current_process_group() -> std::io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        rustix::process::kill_current_process_group(rustix::process::Signal::KILL)
            .map_err(|source| std::io::Error::from_raw_os_error(source.raw_os_error()))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "process-group signals require Linux or macOS",
        ))
    }
}

fn signal_process_group(process_group: u32) -> Result<ProcessGroupSignal, AviateSupervisorError> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let raw = i32::try_from(process_group).map_err(|_| {
            AviateSupervisorError::identity_mismatch(
                "the target process group exceeds POSIX limits",
            )
        })?;
        let group = rustix::process::Pid::from_raw(raw).ok_or_else(|| {
            AviateSupervisorError::identity_mismatch("the target process group is zero")
        })?;
        match rustix::process::kill_process_group(group, rustix::process::Signal::KILL) {
            Ok(()) => Ok(ProcessGroupSignal::Delivered),
            Err(rustix::io::Errno::SRCH) => Ok(ProcessGroupSignal::GroupMissing),
            Err(rustix::io::Errno::PERM) => Ok(ProcessGroupSignal::Refused),
            Err(source) => Err(AviateSupervisorError::ProcessIo {
                operation: "signal exact target process group",
                source: std::io::Error::from_raw_os_error(source.raw_os_error()),
            }),
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = process_group;
        Err(AviateSupervisorError::UnsupportedPlatform)
    }
}

#[cfg(test)]
mod tests;
