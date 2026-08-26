use std::time::{Duration, Instant};

use crate::AviateSupervisorError;
use crate::document::{ProcessIdentity, ProcessStartIdentity};
use crate::inspection::{LifetimeIdentity, ProcessGroupSnapshot};

use super::RecoveryState;

pub(super) fn recover_same_boot(
    state: &RecoveryState,
    timeout: Duration,
) -> Result<(), AviateSupervisorError> {
    let target = state
        .target
        .as_ref()
        .map(|evidence| &evidence.attestation.target);
    let mut wait = SystemWaitControl;
    recover_processes_same_boot_blocking(
        &state.processes.supervisor,
        &state.processes.target_gate,
        target,
        timeout,
        &mut wait,
    )
}

fn recover_processes_same_boot_blocking(
    owner: &ProcessIdentity,
    gate: &ProcessIdentity,
    target: Option<&ProcessIdentity>,
    timeout: Duration,
    wait: &mut impl WaitControl,
) -> Result<(), AviateSupervisorError> {
    let deadline = wait
        .now()
        .checked_add(timeout)
        .ok_or(AviateSupervisorError::Timeout {
            operation: "wait for recovered process removal",
        })?;
    wait_for_owner_ended(owner, deadline, wait)?;
    let snapshot = crate::inspection::process_group_snapshot(gate.process_group)?;
    if !snapshot.is_empty() {
        validate_group(gate, &snapshot)?;
        if !snapshot.is_quiescent() {
            require_live_group_anchor(gate, target, &snapshot)?;
        }
    }
    wait_for_group_empty(gate, deadline, wait)?;
    require_process_ended(gate, "launch gate")?;
    if let Some(target) = target {
        require_process_ended(target, "target")?;
    }
    require_group_absent(gate.process_group)?;
    Ok(())
}

trait WaitControl {
    fn now(&mut self) -> Instant;
    fn park_for_poll_blocking(&mut self, duration: Duration);
}

struct SystemWaitControl;

impl WaitControl for SystemWaitControl {
    fn now(&mut self) -> Instant {
        Instant::now()
    }

    fn park_for_poll_blocking(&mut self, duration: Duration) {
        std::thread::park_timeout(duration.min(Duration::from_millis(1)));
    }
}

pub(super) fn cleanup_resources(state: &RecoveryState) -> Result<(), AviateSupervisorError> {
    crate::artifact::validate_directory(&state.intent.runtime_root, true)?;
    crate::runtime_files::remove_exact_socket(&crate::runtime_files::socket_path(
        &state.intent.runtime_root.path,
        crate::runtime_files::PARENT_READY_SOCKET,
    ))?;
    crate::runtime_files::require_entries(&state.intent.runtime_root.path, &[])?;
    cleanup_artifacts(state)
}

pub(super) fn verify_resources_clean(state: &RecoveryState) -> Result<(), AviateSupervisorError> {
    crate::artifact::validate_directory(&state.intent.runtime_root, true)?;
    crate::runtime_files::require_entries(&state.intent.runtime_root.path, &[])?;
    if path_exists(&state.intent.artifact_root.path)? {
        return Err(AviateSupervisorError::RecoveryBlocked {
            detail: "the launch-artifact root exists after the durable outcome".to_owned(),
        });
    }
    Ok(())
}

fn cleanup_artifacts(state: &RecoveryState) -> Result<(), AviateSupervisorError> {
    if !path_exists(&state.intent.artifact_root.path)? {
        return crate::artifact::stabilize_absent_artifact_root(&state.intent.artifact_root);
    }
    crate::artifact::validate_directory(&state.intent.artifact_root, true)?;
    for executable in [
        &state.intent.target_executable,
        &state.intent.supervisor_executable,
    ] {
        if path_exists(&executable.path)? {
            crate::artifact::remove_staged(&state.intent.artifact_root, executable)?;
        }
    }
    crate::artifact::remove_artifact_root(&state.intent.artifact_root)
}

fn path_exists(path: &std::path::Path) -> Result<bool, AviateSupervisorError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(crate::inspection::io_error(
            "inspect recovery resource",
            path,
            source,
        )),
    }
}

fn wait_for_owner_ended(
    expected: &ProcessIdentity,
    deadline: Instant,
    wait: &mut impl WaitControl,
) -> Result<(), AviateSupervisorError> {
    loop {
        match crate::inspection::inspect_lifetime(expected.pid)? {
            Some(actual) if same_incarnation(expected, &actual) => {}
            Some(_) | None => return Ok(()),
        }
        let now = wait.now();
        if now >= deadline {
            return Err(AviateSupervisorError::Timeout {
                operation: "wait for recovered process owner removal",
            });
        }
        wait.park_for_poll_blocking(deadline.saturating_duration_since(now));
    }
}

fn require_process_ended(
    expected: &ProcessIdentity,
    process: &'static str,
) -> Result<(), AviateSupervisorError> {
    match crate::inspection::inspect_lifetime(expected.pid)? {
        Some(actual) if same_incarnation(expected, &actual) => {
            Err(AviateSupervisorError::RecoveryBlocked {
                detail: format!("the exact {process} lifetime is still present"),
            })
        }
        Some(_) | None => Ok(()),
    }
}

fn validate_group(
    gate: &ProcessIdentity,
    snapshot: &ProcessGroupSnapshot,
) -> Result<(), AviateSupervisorError> {
    let invalid = !snapshot.unclassified_pids.is_empty()
        || snapshot.raw_pids.len() != snapshot.observed.len().wrapping_add(snapshot.exited.len())
        || snapshot.observed.iter().any(|member| {
            member.process_group != gate.process_group
                || member.session_id != gate.session_id
                || member.real_user_id != gate.real_user_id
                || member.start.boot_identity() != gate.start.boot_identity()
        });
    if invalid {
        return Err(AviateSupervisorError::RecoveryBlocked {
            detail: "the recovery group has a member outside the isolated launch session"
                .to_owned(),
        });
    }
    Ok(())
}

fn require_live_group_anchor(
    gate: &ProcessIdentity,
    target: Option<&ProcessIdentity>,
    snapshot: &ProcessGroupSnapshot,
) -> Result<(), AviateSupervisorError> {
    let gate_anchor = snapshot
        .observed
        .iter()
        .any(|actual| same_contained_lifetime(gate, actual))
        && exact_live_identity(gate)?;
    let target_anchor = if let Some(expected) = target {
        snapshot
            .observed
            .iter()
            .any(|actual| same_contained_lifetime(expected, actual))
            && exact_live_identity(expected)?
    } else {
        false
    };
    if !gate_anchor && !target_anchor {
        return Err(AviateSupervisorError::RecoveryBlocked {
            detail: "the live recovery group has no exact process anchor".to_owned(),
        });
    }
    Ok(())
}

fn exact_live_identity(expected: &ProcessIdentity) -> Result<bool, AviateSupervisorError> {
    let Some(actual) =
        crate::inspection::inspect_process(expected.pid, expected.launch_argv_digest)?
    else {
        return Ok(false);
    };
    if !same_recovery_identity(expected, &actual) {
        return Err(AviateSupervisorError::identity_mismatch(
            "a live recovery anchor changed executable identity",
        ));
    }
    Ok(true)
}

fn same_recovery_identity(expected: &ProcessIdentity, actual: &ProcessIdentity) -> bool {
    expected.pid == actual.pid
        && expected.process_group == actual.process_group
        && expected.session_id == actual.session_id
        && expected.real_user_id == actual.real_user_id
        && expected.start == actual.start
        && expected.executable == actual.executable
        && expected.executable_digest == actual.executable_digest
        && expected.launch_argv_digest == actual.launch_argv_digest
        && expected.observed_argv_digest == actual.observed_argv_digest
}

fn wait_for_group_empty(
    gate: &ProcessIdentity,
    deadline: Instant,
    wait: &mut impl WaitControl,
) -> Result<(), AviateSupervisorError> {
    loop {
        let snapshot = crate::inspection::process_group_snapshot(gate.process_group)?;
        if snapshot.is_empty() && crate::inspection::process_group_is_absent(gate.process_group)? {
            return Ok(());
        }
        validate_group(gate, &snapshot)?;
        let now = wait.now();
        if now >= deadline {
            return Err(AviateSupervisorError::Timeout {
                operation: "wait for recovered process group removal",
            });
        }
        wait.park_for_poll_blocking(deadline.saturating_duration_since(now));
    }
}

fn require_group_absent(process_group: u32) -> Result<(), AviateSupervisorError> {
    if crate::inspection::process_group_is_absent(process_group)? {
        Ok(())
    } else {
        Err(AviateSupervisorError::RecoveryBlocked {
            detail: "the exact recovery process group is still present".to_owned(),
        })
    }
}

fn same_contained_lifetime(expected: &ProcessIdentity, actual: &LifetimeIdentity) -> bool {
    same_incarnation(expected, actual)
        && actual.process_group == expected.process_group
        && actual.session_id == expected.session_id
}

fn same_incarnation(expected: &ProcessIdentity, actual: &LifetimeIdentity) -> bool {
    actual.pid == expected.pid
        && actual.real_user_id == expected.real_user_id
        && same_start(&expected.start, &actual.start)
}

fn same_start(expected: &ProcessStartIdentity, actual: &ProcessStartIdentity) -> bool {
    expected == actual
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests;
