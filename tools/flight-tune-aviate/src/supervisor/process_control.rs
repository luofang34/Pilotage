use std::process::Child;

use crate::AviateSupervisorError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ProcessGroupSignal {
    Delivered,
    GroupMissing,
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

pub(super) fn signal_process_group(
    process_group: u32,
) -> Result<ProcessGroupSignal, AviateSupervisorError> {
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
