use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::super::PreparedOwner;
use crate::AviateSupervisorError;
use crate::document::{BootIdentity, ProcessIdentity, SCHEMA_VERSION, TargetAttestation};
use crate::supervisor::SupervisorBootstrap;

#[derive(Clone, Debug, Eq, PartialEq)]
struct TargetExpectation {
    parent_pid: u32,
    process_group: u32,
    session_id: u32,
    real_user_id: u32,
    boot_identity: BootIdentity,
    executable: PathBuf,
    executable_digest: flight_tune::Digest,
}

pub(super) fn attest(
    owner: &mut PreparedOwner,
    pid: u32,
) -> Result<TargetAttestation, AviateSupervisorError> {
    let expectation = TargetExpectation::from_owner(owner);
    let target = wait_for_expected_process(
        pid,
        argv_digest(&owner.bootstrap)?,
        &expectation,
        Duration::from_millis(owner.bootstrap.startup_timeout_millis),
    )?;
    owner.target_identity = Some(target.clone());
    Ok(TargetAttestation {
        schema_version: SCHEMA_VERSION,
        run_intent_digest: owner.bootstrap.run_intent_digest,
        target,
    })
}

pub(super) fn argv_digest(
    bootstrap: &SupervisorBootstrap,
) -> Result<flight_tune::Digest, AviateSupervisorError> {
    let executable =
        bootstrap.target_executable.path.to_str().ok_or_else(|| {
            AviateSupervisorError::invalid_request("the target path is not UTF-8")
        })?;
    let mut arguments = Vec::with_capacity(bootstrap.target_arguments.len().wrapping_add(1));
    arguments.push(executable.to_owned());
    arguments.extend(bootstrap.target_arguments.iter().cloned());
    Ok(crate::inspection::digest_arguments(&arguments))
}

impl TargetExpectation {
    fn from_owner(owner: &PreparedOwner) -> Self {
        let gate = &owner.process_identity.target_gate;
        Self {
            parent_pid: gate.pid,
            process_group: gate.process_group,
            session_id: gate.session_id,
            real_user_id: gate.real_user_id,
            boot_identity: gate.start.boot_identity(),
            executable: owner.bootstrap.target_executable.path.clone(),
            executable_digest: owner.bootstrap.target_executable.digest,
        }
    }
}

fn wait_for_expected_process(
    pid: u32,
    argv_digest: flight_tune::Digest,
    expected: &TargetExpectation,
    timeout: Duration,
) -> Result<ProcessIdentity, AviateSupervisorError> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or(AviateSupervisorError::Timeout {
            operation: "inspect authorized target",
        })?;
    loop {
        if let Some(identity) = crate::inspection::inspect_process_before(
            pid,
            argv_digest,
            deadline,
            "inspect authorized target",
        )? {
            validate_target(&identity, expected)?;
            return Ok(identity);
        }
        if Instant::now() >= deadline {
            return Err(AviateSupervisorError::Timeout {
                operation: "inspect authorized target",
            });
        }
        std::thread::park_timeout(Duration::from_millis(1));
    }
}

fn validate_target(
    actual: &ProcessIdentity,
    expected: &TargetExpectation,
) -> Result<(), AviateSupervisorError> {
    let detail = if actual.parent_pid != expected.parent_pid {
        Some("the target parent process differs from the launch gate")
    } else if actual.process_group != expected.process_group {
        Some("the target process group differs from the launch gate")
    } else if actual.session_id != expected.session_id {
        Some("the target session differs from the launch gate")
    } else if actual.real_user_id != expected.real_user_id {
        Some("the target user differs from the launch gate")
    } else if actual.start.boot_identity() != expected.boot_identity {
        Some("the target boot identity differs from the launch gate")
    } else if actual.executable != expected.executable {
        Some("the target executable path differs from the authorized launch")
    } else if actual.executable_digest != expected.executable_digest {
        Some("the target executable digest differs from the authorized launch")
    } else {
        None
    };
    detail.map_or(Ok(()), |detail| {
        Err(AviateSupervisorError::identity_mismatch(detail))
    })
}
