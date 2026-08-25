use std::io::Write as _;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;

use super::config::{GateConfig, validate_gate_config};
use super::process_control;
use crate::AviateSupervisorError;
use crate::artifact;
use crate::lease_store;
use crate::protocol::{GateEvent, decode, encode_line, read_line_blocking};

enum GateWait {
    SupervisorClosed,
}

pub(super) fn run() -> Result<(), AviateSupervisorError> {
    require_exact_arguments()?;
    enter_private_session()?;
    let mut input = std::io::stdin();
    let config: GateConfig = decode(&read_line_blocking(&mut input)?)?;
    validate_gate_config(&config)?;
    validate_launch_context(&config)?;
    let release = read_line_blocking(&mut input)?;
    if lease_store::digest_bytes(&release) != config.release_secret_digest {
        return Err(AviateSupervisorError::protocol(
            "the anonymous release capability is invalid",
        ));
    }
    let mut target = spawn_target(&config)?;
    let supervision = supervise_spawned_target(&mut target, input);
    match supervision {
        Ok(()) => {
            if let Err(error) = contain_spawned_target(&mut target) {
                hold_failed_containment(target, error);
            }
            if let Err(error) = report_contained_target(&target) {
                tracing::error!(%error, "launch-gate containment report failed");
                signal_own_group();
            }
            hold_contained_group();
        }
        Err(error) => {
            tracing::error!(%error, "launch-gate supervision failed");
            if let Err(error) = contain_spawned_target(&mut target) {
                hold_failed_containment(target, error);
            }
            if let Err(error) = report_contained_target(&target) {
                tracing::error!(%error, "launch-gate containment report failed");
            }
            signal_own_group();
        }
    }
}

fn enter_private_session() -> Result<(), AviateSupervisorError> {
    let session = rustix::process::setsid().map_err(|source| {
        process_io(
            "create isolated launch-gate session",
            std::io::Error::from_raw_os_error(source.raw_os_error()),
        )
    })?;
    let own_pid = i32::try_from(std::process::id()).map_err(|_| {
        AviateSupervisorError::identity_mismatch("the launch-gate PID exceeds POSIX limits")
    })?;
    if session.as_raw_pid() != own_pid {
        return Err(AviateSupervisorError::identity_mismatch(
            "the launch gate did not become its session leader",
        ));
    }
    Ok(())
}

fn supervise_spawned_target(
    target: &mut Child,
    input: std::io::Stdin,
) -> Result<(), AviateSupervisorError> {
    write_event(&GateEvent::TargetStarted { pid: target.id() })?;
    let (sender, receiver) = mpsc::channel();
    spawn_input_monitor(input, sender)?;
    match receiver
        .recv()
        .map_err(|_| AviateSupervisorError::protocol("the launch-gate event channel closed"))?
    {
        GateWait::SupervisorClosed => Ok(()),
    }
}

fn require_exact_arguments() -> Result<(), AviateSupervisorError> {
    if std::env::args_os().count() != 2 {
        return Err(AviateSupervisorError::invalid_request(
            "the launch gate received unexpected command arguments",
        ));
    }
    Ok(())
}

fn validate_launch_context(config: &GateConfig) -> Result<(), AviateSupervisorError> {
    artifact::validate_directory(&config.artifact_root, true)?;
    artifact::validate_directory(&config.target_current_directory, false)?;
    let target = artifact::inspect_staged(
        &config.target_executable.path,
        config.target_executable.digest,
    )?;
    if target != config.target_executable {
        return Err(AviateSupervisorError::identity_mismatch(
            "a launch-gate executable identity changed",
        ));
    }
    Ok(())
}

fn spawn_target(config: &GateConfig) -> Result<Child, AviateSupervisorError> {
    artifact::validate_directory(&config.target_current_directory, false)?;
    let mut command = Command::new(&config.target_executable.path);
    command
        .args(&config.target_arguments)
        .env_clear()
        .envs(&config.target_environment)
        .current_dir(&config.target_current_directory.path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
        .spawn()
        .map_err(|source| process_io("spawn exact supervised target", source))
}

fn spawn_input_monitor(
    mut input: std::io::Stdin,
    sender: mpsc::Sender<GateWait>,
) -> Result<(), AviateSupervisorError> {
    std::thread::Builder::new()
        .name("aviate-gate-owner-eof".to_owned())
        .spawn(move || {
            drop(read_line_blocking(&mut input).ok());
            if sender.send(GateWait::SupervisorClosed).is_err() {
                tracing::debug!("launch-gate owner channel is already closed");
            }
        })
        .map(|_| ())
        .map_err(|source| process_io("spawn launch-gate owner monitor", source))
}

fn write_event(event: &GateEvent) -> Result<(), AviateSupervisorError> {
    let bytes = encode_line(event)?;
    let mut output = std::io::stdout().lock();
    output
        .write_all(&bytes)
        .and_then(|()| output.flush())
        .map_err(|source| process_io("write launch-gate event", source))
}

fn contain_spawned_target(target: &mut Child) -> Result<(), AviateSupervisorError> {
    match process_control::stop_child(target) {
        Ok(()) => tracing::debug!(target_pid = target.id(), "launch gate stopped exact target"),
        Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => {
            tracing::debug!(
                target_pid = target.id(),
                "launch-gate target already stopped"
            );
        }
        Err(error) => {
            return Err(process_io("stop exact supervised target", error));
        }
    }
    target
        .wait()
        .map(|_| ())
        .map_err(|source| process_io("reap exact supervised target", source))
}

fn report_contained_target(target: &Child) -> Result<(), AviateSupervisorError> {
    write_event(&GateEvent::TargetContained { pid: target.id() })
}

fn hold_failed_containment(_target: Child, error: AviateSupervisorError) -> ! {
    tracing::error!(%error, "launch gate uses group containment after exact target cleanup failed");
    signal_own_group()
}

fn hold_contained_group() -> ! {
    loop {
        std::thread::park();
    }
}

fn signal_own_group() -> ! {
    loop {
        match process_control::signal_current_process_group() {
            Ok(()) => tracing::error!("launch-gate containment signal returned"),
            Err(error) => tracing::error!(%error, "launch-gate containment signal failed"),
        }
        std::thread::park_timeout(std::time::Duration::from_millis(100));
    }
}

fn process_io(operation: &'static str, source: std::io::Error) -> AviateSupervisorError {
    AviateSupervisorError::ProcessIo { operation, source }
}
