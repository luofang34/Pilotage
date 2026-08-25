use std::path::Path;

use super::SupervisedProcessRequest;
use crate::AviateSupervisorError;
use crate::artifact;
use crate::document::{
    PROCESS_IDENTITY_NAME, ProcessIdentity, ProcessIdentityDocument, SCHEMA_VERSION,
    SPAWN_INTENT_NAME, SpawnIntent, TARGET_ATTESTATION_NAME, TargetAttestation, TargetStdio,
};
use crate::inspection::{LifetimeIdentity, ProcessGroupSnapshot};
use crate::lease_store::{digest_bytes, read_without_writer};
use crate::process::{RecoveryRequest, SupervisionAttestation};
use crate::protocol::{ArmedMessage, TargetReadyMessage};

pub(super) fn validate_armed(
    request: &SupervisedProcessRequest,
    owner_pid: u32,
    message: &ArmedMessage,
    release_secret: &str,
) -> Result<SupervisionAttestation, AviateSupervisorError> {
    let evidence = read_supervision_evidence(request)?;
    if message.correlation_nonce != evidence.processes.correlation_nonce
        || message.spawn_intent_digest != evidence.intent_digest
        || message.process_identity_digest != evidence.process_digest
        || digest_bytes(release_secret.as_bytes()) != evidence.intent.release_secret_digest
    {
        return Err(invalid_process_document(
            "the armed message differs from the durable supervision evidence",
        ));
    }
    validate_supervision_live(request, owner_pid, &evidence)?;
    let snapshot =
        crate::inspection::process_group_snapshot(evidence.processes.target_gate.process_group)?;
    validate_armed_group(&snapshot, &evidence.processes.target_gate)?;
    Ok(SupervisionAttestation {
        schema_version: crate::process::SUPERVISION_ATTESTATION_SCHEMA_VERSION,
        run_intent_digest: request.run_intent_digest,
        spawn_intent_digest: evidence.intent_digest,
        process_identity_digest: evidence.process_digest,
        supervisor_identity: evidence.processes.supervisor,
        target_gate_identity: evidence.processes.target_gate,
        recovery_request: RecoveryRequest {
            schema_version: crate::process::RECOVERY_REQUEST_SCHEMA_VERSION,
            storage_root: request.storage_root.clone(),
            run_intent_digest: request.run_intent_digest,
            supervisor_executable_digest: request.supervisor_executable_digest,
            target_executable_digest: request.target_executable_digest,
            expected_spawn_intent_digest: evidence.intent_digest,
            expected_process_identity_digest: evidence.process_digest,
            cleanup_timeout_millis: duration_millis(request.cleanup_timeout)?,
        },
    })
}

pub(super) fn validate_target_ready(
    request: &SupervisedProcessRequest,
    message: &TargetReadyMessage,
    expected_supervisor: &ProcessIdentity,
) -> Result<ProcessIdentity, AviateSupervisorError> {
    let evidence = read_supervision_evidence(request)?;
    validate_supervision_live(request, expected_supervisor.pid, &evidence)?;
    if evidence.processes.supervisor != *expected_supervisor
        || message.correlation_nonce != evidence.processes.correlation_nonce
        || message.process_identity_digest != evidence.process_digest
    {
        return Err(invalid_process_document(
            "the target-ready message differs from the durable supervision evidence",
        ));
    }
    let (attestation, digest): (TargetAttestation, _) =
        read_without_writer(&request.storage_root, TARGET_ATTESTATION_NAME)?;
    if digest != message.target_attestation_digest {
        return Err(invalid_process_document(
            "the target-ready message has a different target attestation digest",
        ));
    }
    validate_target_attestation(request, &evidence, &attestation)?;
    Ok(attestation.target)
}

struct SupervisionEvidence {
    intent: SpawnIntent,
    intent_digest: flight_tune::Digest,
    processes: ProcessIdentityDocument,
    process_digest: flight_tune::Digest,
}

fn read_supervision_evidence(
    request: &SupervisedProcessRequest,
) -> Result<SupervisionEvidence, AviateSupervisorError> {
    let (intent, intent_digest) = read_without_writer(&request.storage_root, SPAWN_INTENT_NAME)?;
    validate_spawn_intent(request, &intent)?;
    let (processes, process_digest) =
        read_without_writer(&request.storage_root, PROCESS_IDENTITY_NAME)?;
    validate_process_document(request, &intent, intent_digest, &processes)?;
    Ok(SupervisionEvidence {
        intent,
        intent_digest,
        processes,
        process_digest,
    })
}

fn validate_spawn_intent(
    request: &SupervisedProcessRequest,
    intent: &SpawnIntent,
) -> Result<(), AviateSupervisorError> {
    let target_argv =
        target_argv_digest(&intent.target_executable.path, &request.target_arguments)?;
    let environment = crate::supervisor::config::digest_environment(&request.target_environment);
    let current_directory = artifact::inspect_directory(&request.target_current_directory, false)?;
    let runtime = artifact::inspect_directory(&request.runtime_root, true)?;
    if intent.schema_version != SCHEMA_VERSION
        || intent.run_intent_digest != request.run_intent_digest
        || intent.supervisor_executable.digest != request.supervisor_executable_digest
        || intent.target_executable.digest != request.target_executable_digest
        || intent.target_arguments != request.target_arguments
        || intent.target_argv_digest != target_argv
        || intent.target_environment_digest != environment
        || intent.target_current_directory != current_directory
        || intent.runtime_root != runtime
        || intent.artifact_root.path != request.artifact_root
        || intent.target_stdio != TargetStdio::Null
        || intent.target_process_contract != request.target_process_contract
        || intent.cleanup_timeout_millis != duration_millis(request.cleanup_timeout)?
        || intent.correlation_nonce.is_zero()
        || intent.release_secret_digest.is_zero()
    {
        return Err(invalid_spawn_intent(
            "the durable spawn intent differs from the launch request",
        ));
    }
    validate_staged_artifacts(intent)
}

fn duration_millis(duration: std::time::Duration) -> Result<u64, AviateSupervisorError> {
    u64::try_from(duration.as_millis()).map_err(|_| {
        AviateSupervisorError::invalid_request("the cleanup timeout exceeds its encoded limit")
    })
}

fn validate_staged_artifacts(intent: &SpawnIntent) -> Result<(), AviateSupervisorError> {
    artifact::validate_directory(&intent.artifact_root, true)?;
    for (name, expected) in [
        (
            crate::artifact::SUPERVISOR_ARTIFACT,
            &intent.supervisor_executable,
        ),
        (crate::artifact::TARGET_ARTIFACT, &intent.target_executable),
    ] {
        if expected.path != intent.artifact_root.path.join(name)
            || artifact::inspect_staged(&expected.path, expected.digest)? != *expected
        {
            return Err(AviateSupervisorError::identity_mismatch(
                "a durable staged executable identity changed",
            ));
        }
    }
    Ok(())
}

fn validate_process_document(
    request: &SupervisedProcessRequest,
    intent: &SpawnIntent,
    intent_digest: flight_tune::Digest,
    processes: &ProcessIdentityDocument,
) -> Result<(), AviateSupervisorError> {
    let parent = current_lifetime()?;
    let owner_argv = owner_argv_digest(&intent.supervisor_executable.path, request)?;
    let gate_argv = gate_argv_digest(&intent.supervisor_executable.path)?;
    if processes.schema_version != SCHEMA_VERSION
        || processes.run_intent_digest != request.run_intent_digest
        || processes.spawn_intent_digest != intent_digest
        || processes.correlation_nonce != intent.correlation_nonce
        || processes.supervisor.parent_pid != std::process::id()
        || processes.supervisor.session_id != parent.session_id
        || processes.supervisor.real_user_id != parent.real_user_id
        || processes.supervisor.start.boot_identity() != parent.start.boot_identity()
        || processes.supervisor.executable != intent.supervisor_executable.path
        || processes.supervisor.executable_digest != intent.supervisor_executable.digest
        || processes.supervisor.launch_argv_digest != owner_argv
        || processes.target_gate.parent_pid != processes.supervisor.pid
        || processes.target_gate.pid != processes.target_gate.process_group
        || processes.target_gate.pid != processes.target_gate.session_id
        || processes.target_gate.real_user_id != parent.real_user_id
        || processes.target_gate.start.boot_identity() != parent.start.boot_identity()
        || processes.target_gate.executable != intent.supervisor_executable.path
        || processes.target_gate.executable_digest != intent.supervisor_executable.digest
        || processes.target_gate.launch_argv_digest != gate_argv
    {
        return Err(invalid_process_document(
            "the durable owner or launch-gate identity is invalid",
        ));
    }
    Ok(())
}

fn validate_supervision_live(
    request: &SupervisedProcessRequest,
    owner_pid: u32,
    evidence: &SupervisionEvidence,
) -> Result<(), AviateSupervisorError> {
    if evidence.processes.supervisor.pid != owner_pid {
        return Err(invalid_process_document(
            "the durable owner PID differs from the held child",
        ));
    }
    validate_live_process(&evidence.processes.supervisor, "owner")?;
    validate_live_process(&evidence.processes.target_gate, "launch gate")?;
    validate_process_document(
        request,
        &evidence.intent,
        evidence.intent_digest,
        &evidence.processes,
    )
}

fn validate_live_process(
    expected: &ProcessIdentity,
    process: &'static str,
) -> Result<(), AviateSupervisorError> {
    let actual = crate::inspection::inspect_process(expected.pid, expected.launch_argv_digest)?
        .ok_or_else(|| {
            AviateSupervisorError::identity_mismatch(format!("the exact {process} is absent"))
        })?;
    crate::inspection::validate_same_lifetime(expected, &actual, process)?;
    if actual != *expected {
        return Err(AviateSupervisorError::identity_mismatch(format!(
            "the exact {process} identity changed"
        )));
    }
    let lifetime = crate::inspection::inspect_lifetime(expected.pid)?.ok_or_else(|| {
        AviateSupervisorError::identity_mismatch(format!("the exact {process} is absent"))
    })?;
    if lifetime.is_zombie {
        return Err(AviateSupervisorError::identity_mismatch(format!(
            "the exact {process} is a zombie"
        )));
    }
    Ok(())
}

fn validate_armed_group(
    snapshot: &ProcessGroupSnapshot,
    expected_gate: &ProcessIdentity,
) -> Result<(), AviateSupervisorError> {
    if snapshot.raw_pids != [expected_gate.pid]
        || !snapshot.exited.is_empty()
        || !snapshot.unclassified_pids.is_empty()
        || snapshot.observed.len() != 1
        || !same_lifetime(&snapshot.observed[0], expected_gate)
        || snapshot.observed[0].is_zombie
    {
        return Err(AviateSupervisorError::identity_mismatch(
            "the armed launch-gate group has unexpected members",
        ));
    }
    Ok(())
}

fn validate_target_attestation(
    request: &SupervisedProcessRequest,
    evidence: &SupervisionEvidence,
    attestation: &TargetAttestation,
) -> Result<(), AviateSupervisorError> {
    let target = &attestation.target;
    if attestation.schema_version != SCHEMA_VERSION
        || attestation.run_intent_digest != request.run_intent_digest
        || target.parent_pid != evidence.processes.target_gate.pid
        || target.process_group != evidence.processes.target_gate.process_group
        || target.session_id != evidence.processes.target_gate.session_id
        || target.real_user_id != evidence.processes.target_gate.real_user_id
        || target.start.boot_identity() != evidence.processes.target_gate.start.boot_identity()
        || target.executable != evidence.intent.target_executable.path
        || target.executable_digest != request.target_executable_digest
        || target.launch_argv_digest != evidence.intent.target_argv_digest
    {
        return Err(invalid_process_document(
            "the target attestation differs from the authorized launch",
        ));
    }
    validate_live_process(target, "target")?;
    let snapshot = crate::inspection::process_group_snapshot(target.process_group)?;
    validate_ready_group(&snapshot, &evidence.processes.target_gate, target)
}

fn validate_ready_group(
    snapshot: &ProcessGroupSnapshot,
    gate: &ProcessIdentity,
    target: &ProcessIdentity,
) -> Result<(), AviateSupervisorError> {
    let has_gate = snapshot
        .observed
        .iter()
        .any(|actual| same_lifetime(actual, gate));
    let has_target = snapshot
        .observed
        .iter()
        .any(|actual| same_lifetime(actual, target));
    if !has_gate
        || !has_target
        || !snapshot.exited.is_empty()
        || !snapshot.unclassified_pids.is_empty()
        || snapshot.observed.iter().any(|member| {
            member.is_zombie
                || member.session_id != gate.session_id
                || member.real_user_id != gate.real_user_id
        })
    {
        return Err(AviateSupervisorError::identity_mismatch(
            "the ready target group is incomplete or not live",
        ));
    }
    Ok(())
}

fn current_lifetime() -> Result<LifetimeIdentity, AviateSupervisorError> {
    crate::inspection::inspect_lifetime(std::process::id())?.ok_or_else(|| {
        AviateSupervisorError::identity_mismatch("the parent process identity is unavailable")
    })
}

fn same_lifetime(actual: &LifetimeIdentity, expected: &ProcessIdentity) -> bool {
    actual.pid == expected.pid
        && actual.process_group == expected.process_group
        && actual.session_id == expected.session_id
        && actual.parent_pid == expected.parent_pid
        && actual.real_user_id == expected.real_user_id
        && actual.start == expected.start
}

fn owner_argv_digest(
    executable: &Path,
    request: &SupervisedProcessRequest,
) -> Result<flight_tune::Digest, AviateSupervisorError> {
    argv_digest(
        executable,
        &["supervise"],
        &[&request.storage_root, &request.runtime_root],
    )
}

fn gate_argv_digest(executable: &Path) -> Result<flight_tune::Digest, AviateSupervisorError> {
    argv_digest(executable, &["gate"], &[])
}

fn argv_digest(
    executable: &Path,
    fixed: &[&str],
    paths: &[&Path],
) -> Result<flight_tune::Digest, AviateSupervisorError> {
    let mut arguments = vec![utf8_path(executable)?];
    arguments.extend(fixed.iter().map(|value| (*value).to_owned()));
    for path in paths {
        arguments.push(utf8_path(path)?);
    }
    Ok(crate::inspection::digest_arguments(&arguments))
}

fn target_argv_digest(
    executable: &Path,
    arguments: &[String],
) -> Result<flight_tune::Digest, AviateSupervisorError> {
    let mut argv = Vec::with_capacity(arguments.len().wrapping_add(1));
    argv.push(utf8_path(executable)?);
    argv.extend(arguments.iter().cloned());
    Ok(crate::inspection::digest_arguments(&argv))
}

fn utf8_path(path: &Path) -> Result<String, AviateSupervisorError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| AviateSupervisorError::invalid_request("a launch path is not UTF-8"))
}

fn invalid_spawn_intent(detail: &'static str) -> AviateSupervisorError {
    AviateSupervisorError::invalid_document("spawn intent", detail)
}

fn invalid_process_document(detail: &'static str) -> AviateSupervisorError {
    AviateSupervisorError::invalid_document("process identity", detail)
}
