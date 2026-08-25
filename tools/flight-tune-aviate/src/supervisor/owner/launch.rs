use std::io::{BufRead as _, Read as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::super::config::{
    BOOTSTRAP_SCHEMA_VERSION, GateConfig, SupervisorBootstrap, digest_environment,
    validate_bootstrap,
};
use super::super::process_control;
use super::{OwnerEvent, PreparedOwner, TargetPublicationState, cleanup, handshake, process_io};
use crate::AviateSupervisorError;
use crate::artifact;
use crate::document::{
    ProcessIdentity, ProcessIdentityDocument, SCHEMA_VERSION, SPAWN_INTENT_NAME, SpawnIntent,
    TargetAttestation, TargetStdio,
};
use crate::inspection;
use crate::lease_store::LeaseStore;
use crate::protocol::{GateEvent, ReleaseMessage, decode, read_line_blocking};
use crate::runtime_files::PARENT_READY_SOCKET;

#[path = "launch/target.rs"]
mod target;

pub(super) fn attest_target(
    owner: &mut PreparedOwner,
    pid: u32,
) -> Result<TargetAttestation, AviateSupervisorError> {
    target::attest(owner, pid)
}

pub(super) fn prepare() -> Result<PreparedOwner, AviateSupervisorError> {
    let (storage_root, runtime_root) = parse_paths()?;
    let mut input = std::io::stdin();
    let bootstrap: SupervisorBootstrap = decode(&read_line_blocking(&mut input)?)?;
    validate_bootstrap(&bootstrap)?;
    validate_paths(&bootstrap, &runtime_root)?;
    validate_artifacts(&bootstrap)?;
    let (event_sender, events) = start_event_channel(input)?;
    prepare_owner(storage_root, bootstrap, event_sender, events)
}

fn prepare_owner(
    storage_root: PathBuf,
    bootstrap: SupervisorBootstrap,
    event_sender: mpsc::Sender<OwnerEvent>,
    events: mpsc::Receiver<OwnerEvent>,
) -> Result<PreparedOwner, AviateSupervisorError> {
    let store = LeaseStore::create_fresh(&storage_root)?;
    let correlation_nonce = random_digest()?;
    let release_secret_digest = bootstrap.release_secret_digest;
    let intent = spawn_intent(&bootstrap, correlation_nonce, release_secret_digest)?;
    let spawn_intent_digest = store.publish(SPAWN_INTENT_NAME, &intent)?;
    let self_identity = inspect_self(&bootstrap)?;
    let (gate, gate_input, gate_output, gate_identity) = spawn_gate(&bootstrap)?;
    let identity = ProcessIdentityDocument {
        schema_version: SCHEMA_VERSION,
        run_intent_digest: bootstrap.run_intent_digest,
        spawn_intent_digest,
        correlation_nonce,
        supervisor: self_identity,
        target_gate: gate_identity,
    };
    let mut owner = PreparedOwner {
        bootstrap,
        store,
        spawn_intent_digest,
        process_identity_digest: None,
        target_publication: TargetPublicationState::NotAttempted,
        process_identity: identity,
        target_pid: None,
        target_release_sent: false,
        target_identity: None,
        target_contained: false,
        gate_failed: false,
        gate,
        gate_input: Some(gate_input),
        events,
    };
    if let Err(error) = start_gate_monitor(gate_output, event_sender) {
        return cleanup::finish_after_error(owner, error);
    }
    if let Err(error) = handshake::arm_owner(&mut owner, release_secret_digest) {
        return cleanup::finish_after_error(owner, error);
    }
    Ok(owner)
}

fn spawn_intent(
    bootstrap: &SupervisorBootstrap,
    correlation_nonce: flight_tune::Digest,
    release_secret_digest: flight_tune::Digest,
) -> Result<SpawnIntent, AviateSupervisorError> {
    Ok(SpawnIntent {
        schema_version: SCHEMA_VERSION,
        run_intent_digest: bootstrap.run_intent_digest,
        correlation_nonce,
        release_secret_digest,
        supervisor_executable: bootstrap.supervisor_executable.clone(),
        target_executable: bootstrap.target_executable.clone(),
        target_arguments: bootstrap.target_arguments.clone(),
        target_argv_digest: target::argv_digest(bootstrap)?,
        target_environment_digest: digest_environment(&bootstrap.target_environment),
        target_current_directory: bootstrap.target_current_directory.clone(),
        target_stdio: TargetStdio::Null,
        target_process_contract: bootstrap.target_process_contract.clone(),
        cleanup_timeout_millis: bootstrap.cleanup_timeout_millis,
        runtime_root: bootstrap.runtime_root.clone(),
        artifact_root: bootstrap.artifact_root.clone(),
    })
}

pub(super) fn gate_config(
    bootstrap: &SupervisorBootstrap,
    release_secret_digest: flight_tune::Digest,
) -> Result<GateConfig, AviateSupervisorError> {
    Ok(GateConfig {
        schema_version: BOOTSTRAP_SCHEMA_VERSION,
        release_secret_digest,
        target_executable: bootstrap.target_executable.clone(),
        artifact_root: bootstrap.artifact_root.clone(),
        target_arguments: bootstrap.target_arguments.clone(),
        target_environment: bootstrap.target_environment.clone(),
        target_environment_digest: digest_environment(&bootstrap.target_environment),
        target_current_directory: bootstrap.target_current_directory.clone(),
        target_argv_digest: target::argv_digest(bootstrap)?,
        target_process_contract: bootstrap.target_process_contract.clone(),
    })
}

fn spawn_gate(
    bootstrap: &SupervisorBootstrap,
) -> Result<
    (
        Child,
        ChildStdin,
        std::process::ChildStdout,
        ProcessIdentity,
    ),
    AviateSupervisorError,
> {
    let mut command = Command::new(&bootstrap.supervisor_executable.path);
    command
        .arg("gate")
        .env_clear()
        .current_dir(&bootstrap.artifact_root.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let child = command
        .spawn()
        .map_err(|source| process_io("spawn exact target launch gate", source))?;
    finish_gate_spawn(bootstrap, child)
}

fn finish_gate_spawn(
    bootstrap: &SupervisorBootstrap,
    mut child: Child,
) -> Result<
    (
        Child,
        ChildStdin,
        std::process::ChildStdout,
        ProcessIdentity,
    ),
    AviateSupervisorError,
> {
    let result = take_gate_pipes(&mut child).and_then(|(input, output)| {
        let argv = vec![
            bootstrap
                .supervisor_executable
                .path
                .to_string_lossy()
                .into_owned(),
            "gate".to_owned(),
        ];
        let identity = wait_for_gate_process(
            child.id(),
            inspection::digest_arguments(&argv),
            Duration::from_millis(bootstrap.startup_timeout_millis),
        )?;
        validate_gate_identity(bootstrap, &identity)?;
        Ok((input, output, identity))
    });
    match result {
        Ok((input, output, identity)) => Ok((child, input, output, identity)),
        Err(error) => cleanup_unconfigured_gate(child, error),
    }
}

fn wait_for_gate_process(
    pid: u32,
    argv_digest: flight_tune::Digest,
    timeout: Duration,
) -> Result<ProcessIdentity, AviateSupervisorError> {
    let deadline = checked_deadline(timeout, "inspect isolated launch gate")?;
    loop {
        let isolated = match inspection::inspect_lifetime(pid) {
            Ok(Some(identity)) => identity.process_group == pid && identity.session_id == pid,
            Ok(None) | Err(AviateSupervisorError::IdentityMismatch { .. }) => false,
            Err(error) => return Err(error),
        };
        if isolated {
            match inspection::inspect_process_before(
                pid,
                argv_digest,
                deadline,
                "inspect isolated launch gate",
            ) {
                Ok(Some(identity))
                    if identity.process_group == pid && identity.session_id == pid =>
                {
                    return Ok(identity);
                }
                Ok(_) | Err(AviateSupervisorError::IdentityMismatch { .. }) => {}
                Err(error) => return Err(error),
            }
        }
        if Instant::now() >= deadline {
            return Err(AviateSupervisorError::Timeout {
                operation: "inspect isolated launch gate",
            });
        }
        std::thread::park_timeout(Duration::from_millis(1));
    }
}

fn take_gate_pipes(
    child: &mut Child,
) -> Result<(ChildStdin, std::process::ChildStdout), AviateSupervisorError> {
    let input = child
        .stdin
        .take()
        .ok_or_else(|| AviateSupervisorError::protocol("the launch-gate input pipe is missing"))?;
    let output = child
        .stdout
        .take()
        .ok_or_else(|| AviateSupervisorError::protocol("the launch-gate event pipe is missing"))?;
    Ok((input, output))
}

fn validate_gate_identity(
    bootstrap: &SupervisorBootstrap,
    identity: &ProcessIdentity,
) -> Result<(), AviateSupervisorError> {
    if identity.process_group != identity.pid
        || identity.session_id != identity.pid
        || identity.executable != bootstrap.supervisor_executable.path
        || identity.executable_digest != bootstrap.supervisor_executable.digest
        || identity.parent_pid != std::process::id()
    {
        return Err(AviateSupervisorError::identity_mismatch(
            "the target launch-gate identity is invalid",
        ));
    }
    Ok(())
}

fn cleanup_unconfigured_gate<T>(
    mut child: Child,
    source: AviateSupervisorError,
) -> Result<T, AviateSupervisorError> {
    let stop = match process_control::stop_child(&mut child) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => Ok(()),
        Err(error) => Err(process_io("stop unconfigured launch gate", error)),
    };
    let reap = child
        .wait()
        .map(|_| ())
        .map_err(|error| process_io("reap unconfigured launch gate", error));
    let cleanup = match (stop, reap) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(source), Err(cleanup)) => Err(AviateSupervisorError::StartupCleanup {
            source: Box::new(source),
            cleanup: Box::new(cleanup),
        }),
    };
    match cleanup {
        Ok(()) => Err(source),
        Err(cleanup) => Err(AviateSupervisorError::StartupCleanup {
            source: Box::new(source),
            cleanup: Box::new(cleanup),
        }),
    }
}

fn inspect_self(bootstrap: &SupervisorBootstrap) -> Result<ProcessIdentity, AviateSupervisorError> {
    let arguments = std::env::args().collect::<Vec<_>>();
    let identity =
        inspection::inspect_process(std::process::id(), inspection::digest_arguments(&arguments))?
            .ok_or_else(|| {
                AviateSupervisorError::identity_mismatch("the supervisor disappeared")
            })?;
    if identity.executable != bootstrap.supervisor_executable.path
        || identity.executable_digest != bootstrap.supervisor_executable.digest
    {
        return Err(AviateSupervisorError::identity_mismatch(
            "the supervisor executable identity is invalid",
        ));
    }
    Ok(identity)
}

fn validate_artifacts(bootstrap: &SupervisorBootstrap) -> Result<(), AviateSupervisorError> {
    artifact::validate_directory(&bootstrap.artifact_root, true)?;
    artifact::validate_directory(&bootstrap.target_current_directory, false)?;
    for executable in [
        &bootstrap.supervisor_executable,
        &bootstrap.target_executable,
    ] {
        if artifact::inspect_staged(&executable.path, executable.digest)? != *executable {
            return Err(AviateSupervisorError::identity_mismatch(
                "a staged launch executable identity changed",
            ));
        }
    }
    Ok(())
}

fn validate_paths(
    bootstrap: &SupervisorBootstrap,
    runtime_root: &Path,
) -> Result<(), AviateSupervisorError> {
    if runtime_root != bootstrap.runtime_root.path {
        return Err(AviateSupervisorError::identity_mismatch(
            "the runtime path differs from the anonymous bootstrap",
        ));
    }
    crate::runtime_files::validate_private_root(runtime_root)?;
    artifact::validate_directory(&bootstrap.runtime_root, true)?;
    crate::runtime_files::require_entries(runtime_root, &[PARENT_READY_SOCKET])
}

fn parse_paths() -> Result<(PathBuf, PathBuf), AviateSupervisorError> {
    let arguments = std::env::args_os().collect::<Vec<_>>();
    if arguments.len() != 4 {
        return Err(AviateSupervisorError::invalid_request(
            "the process owner received unexpected command arguments",
        ));
    }
    Ok((PathBuf::from(&arguments[2]), PathBuf::from(&arguments[3])))
}

fn start_event_channel(
    mut input: std::io::Stdin,
) -> Result<(mpsc::Sender<OwnerEvent>, mpsc::Receiver<OwnerEvent>), AviateSupervisorError> {
    let (sender, receiver) = mpsc::channel();
    let monitor_sender = sender.clone();
    std::thread::Builder::new()
        .name("aviate-owner-parent-eof".to_owned())
        .spawn(move || monitor_parent(&mut input, &monitor_sender))
        .map(|_| (sender, receiver))
        .map_err(|source| process_io("spawn owner parent monitor", source))
}

fn monitor_parent(input: &mut std::io::Stdin, sender: &mpsc::Sender<OwnerEvent>) {
    match read_line_blocking(input) {
        Ok(bytes) => {
            if sender
                .send(OwnerEvent::ParentRelease(decode::<ReleaseMessage>(&bytes)))
                .is_err()
            {
                return;
            }
        }
        Err(_) => {
            if sender.send(OwnerEvent::ParentClosed).is_err() {
                tracing::debug!("owner parent channel is already closed");
            }
            return;
        }
    }
    let mut remainder = Vec::new();
    if let Err(error) = input.read_to_end(&mut remainder) {
        tracing::error!(%error, "owner parent pipe read failed");
    }
    if sender.send(OwnerEvent::ParentClosed).is_err() {
        tracing::debug!("owner parent channel is already closed");
    }
}

fn start_gate_monitor(
    output: std::process::ChildStdout,
    sender: mpsc::Sender<OwnerEvent>,
) -> Result<(), AviateSupervisorError> {
    std::thread::Builder::new()
        .name("aviate-owner-gate-events".to_owned())
        .spawn(move || monitor_gate(output, &sender))
        .map(|_| ())
        .map_err(|source| process_io("spawn launch-gate event monitor", source))
}

fn monitor_gate(output: std::process::ChildStdout, sender: &mpsc::Sender<OwnerEvent>) {
    let mut reader = std::io::BufReader::new(output);
    loop {
        let mut line = String::new();
        let event = match reader.read_line(&mut line) {
            Ok(0) => Err(AviateSupervisorError::protocol(
                "the launch-gate event pipe closed",
            )),
            Ok(_) => decode::<GateEvent>(line.as_bytes()),
            Err(source) => Err(process_io("read launch-gate event", source)),
        };
        let terminal = event.is_err();
        if sender.send(OwnerEvent::Gate(event)).is_err() || terminal {
            break;
        }
    }
}

fn random_digest() -> Result<flight_tune::Digest, AviateSupervisorError> {
    let path = Path::new("/dev/urandom");
    let mut random = std::fs::File::open(path).map_err(|source| {
        inspection::io_error("open operating-system random source", path, source)
    })?;
    let mut bytes = [0_u8; 32];
    random.read_exact(&mut bytes).map_err(|source| {
        inspection::io_error("read operating-system random source", path, source)
    })?;
    Ok(flight_tune::Digest::from_bytes(bytes))
}

fn checked_deadline(
    timeout: Duration,
    operation: &'static str,
) -> Result<Instant, AviateSupervisorError> {
    Instant::now()
        .checked_add(timeout)
        .ok_or(AviateSupervisorError::Timeout { operation })
}

#[cfg(test)]
mod tests;
