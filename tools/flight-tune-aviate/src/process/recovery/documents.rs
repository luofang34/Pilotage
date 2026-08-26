use crate::AviateSupervisorError;
use crate::document::{
    BootIdentity, PROCESS_IDENTITY_NAME, ProcessIdentity, ProcessIdentityDocument,
    RECOVERY_RECEIPT_NAME, RecoveryReceipt, SCHEMA_VERSION, SPAWN_INTENT_NAME, SpawnIntent,
    TARGET_ATTESTATION_NAME, TERMINAL_RECEIPT_NAME, TargetAttestation, TerminalReceipt,
};
use crate::lease_store::LeaseStore;

use super::{RecoveryOutcome, RecoveryRequest, RecoveryState, TargetEvidence};

const DOCUMENT_NAMES: &[&str] = &[
    SPAWN_INTENT_NAME,
    PROCESS_IDENTITY_NAME,
    TARGET_ATTESTATION_NAME,
    TERMINAL_RECEIPT_NAME,
    RECOVERY_RECEIPT_NAME,
];

pub(super) struct LoadedRecovery {
    pub(super) state: RecoveryState,
    terminal: Option<(TerminalReceipt, flight_tune::Digest)>,
    boot_change: Option<(RecoveryReceipt, flight_tune::Digest)>,
}

pub(super) fn load(
    store: &LeaseStore,
    request: &RecoveryRequest,
) -> Result<LoadedRecovery, AviateSupervisorError> {
    let (intent, intent_digest): (SpawnIntent, _) = store.repair(SPAWN_INTENT_NAME)?;
    let (processes, process_digest): (ProcessIdentityDocument, _) =
        store.repair(PROCESS_IDENTITY_NAME)?;
    validate_intent(request, &intent)?;
    validate_state_digests(request, intent_digest, process_digest)?;
    validate_processes(request, &intent, intent_digest, &processes)?;
    let target = repair_target(store)?;
    let terminal = store.repair_optional(TERMINAL_RECEIPT_NAME)?;
    let boot_change = store.repair_optional(RECOVERY_RECEIPT_NAME)?;
    store.finish_recovery_scan(DOCUMENT_NAMES)?;
    if let Some(target) = &target {
        validate_target(request, &intent, &processes, target)?;
    }
    Ok(LoadedRecovery {
        state: RecoveryState {
            intent,
            intent_digest,
            processes,
            process_digest,
            target,
        },
        terminal,
        boot_change,
    })
}

pub(super) fn existing_outcome(
    request: &RecoveryRequest,
    loaded: &LoadedRecovery,
    current_boot: &BootIdentity,
) -> Result<Option<RecoveryOutcome>, AviateSupervisorError> {
    match (&loaded.terminal, &loaded.boot_change) {
        (Some(_), Some(_)) => Err(AviateSupervisorError::invalid_document(
            "process outcome",
            "terminal and boot-change receipts both exist",
        )),
        (Some((receipt, digest)), None) => {
            validate_terminal(request, &loaded.state, receipt)?;
            Ok(Some(RecoveryOutcome::Terminal {
                receipt_digest: *digest,
            }))
        }
        (None, Some((receipt, digest))) => {
            validate_boot_change(request, &loaded.state, receipt, current_boot)?;
            Ok(Some(RecoveryOutcome::BootChange {
                receipt_digest: *digest,
            }))
        }
        (None, None) => Ok(None),
    }
}

fn repair_target(store: &LeaseStore) -> Result<Option<TargetEvidence>, AviateSupervisorError> {
    let repaired: Option<(TargetAttestation, _)> =
        store.repair_optional(TARGET_ATTESTATION_NAME)?;
    Ok(repaired.map(|(attestation, digest)| TargetEvidence {
        attestation,
        digest,
    }))
}

fn validate_intent(
    request: &RecoveryRequest,
    intent: &SpawnIntent,
) -> Result<(), AviateSupervisorError> {
    let target_path = intent
        .artifact_root
        .path
        .join(crate::artifact::TARGET_ARTIFACT);
    let supervisor_path = intent
        .artifact_root
        .path
        .join(crate::artifact::SUPERVISOR_ARTIFACT);
    if intent.schema_version != SCHEMA_VERSION
        || intent.run_intent_digest != request.run_intent_digest
        || intent.supervisor_executable.digest != request.supervisor_executable_digest
        || intent.target_executable.digest != request.target_executable_digest
        || intent.supervisor_executable.path != supervisor_path
        || intent.target_executable.path != target_path
        || intent.cleanup_timeout_millis != request.cleanup_timeout_millis
        || intent.correlation_nonce.is_zero()
        || intent.release_secret_digest.is_zero()
        || intent.target_argv_digest != target_argv_digest(intent)?
    {
        return Err(AviateSupervisorError::invalid_document(
            "spawn intent",
            "the durable spawn intent differs from the recovery request",
        ));
    }
    Ok(())
}

fn validate_state_digests(
    request: &RecoveryRequest,
    intent_digest: flight_tune::Digest,
    process_digest: flight_tune::Digest,
) -> Result<(), AviateSupervisorError> {
    if intent_digest != request.expected_spawn_intent_digest
        || process_digest != request.expected_process_identity_digest
    {
        return Err(AviateSupervisorError::invalid_document(
            "supervision attestation",
            "the durable supervision digests differ from the recovery request",
        ));
    }
    Ok(())
}

fn validate_processes(
    request: &RecoveryRequest,
    intent: &SpawnIntent,
    intent_digest: flight_tune::Digest,
    processes: &ProcessIdentityDocument,
) -> Result<(), AviateSupervisorError> {
    let owner_argv = owner_argv_digest(request, intent)?;
    let gate_argv = gate_argv_digest(intent)?;
    let owner = &processes.supervisor;
    let gate = &processes.target_gate;
    if processes.schema_version != SCHEMA_VERSION
        || processes.run_intent_digest != request.run_intent_digest
        || processes.spawn_intent_digest != intent_digest
        || processes.correlation_nonce != intent.correlation_nonce
        || owner.executable != intent.supervisor_executable.path
        || owner.executable_digest != request.supervisor_executable_digest
        || !argv_matches(owner, owner_argv)
        || gate.executable != intent.supervisor_executable.path
        || gate.executable_digest != request.supervisor_executable_digest
        || !argv_matches(gate, gate_argv)
        || gate.parent_pid != owner.pid
        || gate.pid == owner.pid
        || gate.pid != gate.process_group
        || gate.pid != gate.session_id
        || gate.real_user_id != owner.real_user_id
        || gate.start.boot_identity() != owner.start.boot_identity()
        || !owner.start.boot_identity().is_valid()
    {
        return Err(AviateSupervisorError::invalid_document(
            "process identity",
            "the durable process identity differs from the recovery request",
        ));
    }
    Ok(())
}

fn validate_target(
    request: &RecoveryRequest,
    intent: &SpawnIntent,
    processes: &ProcessIdentityDocument,
    evidence: &TargetEvidence,
) -> Result<(), AviateSupervisorError> {
    let value = &evidence.attestation;
    let target = &value.target;
    let gate = &processes.target_gate;
    if value.schema_version != SCHEMA_VERSION
        || value.run_intent_digest != request.run_intent_digest
        || target.pid == gate.pid
        || target.parent_pid != gate.pid
        || target.process_group != gate.process_group
        || target.session_id != gate.session_id
        || target.real_user_id != gate.real_user_id
        || target.start.boot_identity() != gate.start.boot_identity()
        || target.executable != intent.target_executable.path
        || target.executable_digest != request.target_executable_digest
        || !argv_matches(target, intent.target_argv_digest)
    {
        return Err(AviateSupervisorError::invalid_document(
            "target attestation",
            "the target attestation differs from the recovery request",
        ));
    }
    Ok(())
}

fn validate_terminal(
    request: &RecoveryRequest,
    state: &RecoveryState,
    terminal: &TerminalReceipt,
) -> Result<(), AviateSupervisorError> {
    let expected_target = state.target.as_ref().map(|target| target.digest);
    if terminal.schema_version != SCHEMA_VERSION
        || terminal.run_intent_digest != request.run_intent_digest
        || terminal.spawn_intent_digest != state.intent_digest
        || terminal.process_identity_digest != state.process_digest
        || terminal.target_attestation_digest != expected_target
    {
        return Err(AviateSupervisorError::invalid_document(
            "terminal receipt",
            "the terminal receipt differs from the recovery request",
        ));
    }
    Ok(())
}

fn validate_boot_change(
    request: &RecoveryRequest,
    state: &RecoveryState,
    receipt: &RecoveryReceipt,
    current_boot: &BootIdentity,
) -> Result<(), AviateSupervisorError> {
    let expected_target = state.target.as_ref().map(|target| target.digest);
    if receipt.schema_version != SCHEMA_VERSION
        || receipt.run_intent_digest != request.run_intent_digest
        || receipt.spawn_intent_digest != state.intent_digest
        || receipt.process_identity_digest != state.process_digest
        || receipt.target_attestation_digest != expected_target
        || receipt.prior_boot_identity != state.processes.supervisor.start.boot_identity()
        || !valid_boot_transition(
            &receipt.prior_boot_identity,
            &receipt.recovery_boot_identity,
            current_boot,
        )
    {
        return Err(AviateSupervisorError::invalid_document(
            "recovery receipt",
            "the boot-change receipt differs from the recovery request",
        ));
    }
    Ok(())
}

fn valid_boot_transition(
    prior: &BootIdentity,
    recovery: &BootIdentity,
    current: &BootIdentity,
) -> bool {
    prior != recovery
        && current != prior
        && prior.is_valid()
        && recovery.is_valid()
        && current.is_valid()
}

fn argv_matches(identity: &ProcessIdentity, expected: flight_tune::Digest) -> bool {
    identity.launch_argv_digest == expected
        && identity
            .observed_argv_digest
            .is_none_or(|actual| actual == expected)
}

fn target_argv_digest(intent: &SpawnIntent) -> Result<flight_tune::Digest, AviateSupervisorError> {
    let mut arguments = Vec::with_capacity(intent.target_arguments.len().wrapping_add(1));
    arguments.push(utf8_path(&intent.target_executable.path)?);
    arguments.extend(intent.target_arguments.iter().cloned());
    Ok(crate::inspection::digest_arguments(&arguments))
}

fn owner_argv_digest(
    request: &RecoveryRequest,
    intent: &SpawnIntent,
) -> Result<flight_tune::Digest, AviateSupervisorError> {
    argv_digest(
        &intent.supervisor_executable.path,
        &["supervise"],
        &[&request.storage_root, &intent.runtime_root.path],
    )
}

fn gate_argv_digest(intent: &SpawnIntent) -> Result<flight_tune::Digest, AviateSupervisorError> {
    argv_digest(&intent.supervisor_executable.path, &["gate"], &[])
}

fn argv_digest(
    executable: &std::path::Path,
    fixed: &[&str],
    paths: &[&std::path::Path],
) -> Result<flight_tune::Digest, AviateSupervisorError> {
    let mut arguments = vec![utf8_path(executable)?];
    arguments.extend(fixed.iter().map(|value| (*value).to_owned()));
    for path in paths {
        arguments.push(utf8_path(path)?);
    }
    Ok(crate::inspection::digest_arguments(&arguments))
}

fn utf8_path(path: &std::path::Path) -> Result<String, AviateSupervisorError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| AviateSupervisorError::invalid_request("a recovery path is not UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::{BootIdentity, valid_boot_transition};

    #[test]
    fn boot_change_replay_rejects_the_prior_boot() {
        let prior = linux_boot("11111111-1111-1111-1111-111111111111");
        let recovery = linux_boot("22222222-2222-2222-2222-222222222222");
        let later = linux_boot("33333333-3333-3333-3333-333333333333");

        assert!(!valid_boot_transition(&prior, &recovery, &prior));
        assert!(valid_boot_transition(&prior, &recovery, &recovery));
        assert!(valid_boot_transition(&prior, &recovery, &later));
        assert!(!valid_boot_transition(&prior, &prior, &later));
        assert!(!valid_boot_transition(
            &linux_boot("invalid"),
            &recovery,
            &later
        ));
    }

    fn linux_boot(value: &str) -> BootIdentity {
        BootIdentity::Linux {
            boot_id: value.to_owned(),
        }
    }
}
