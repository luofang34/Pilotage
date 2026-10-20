//! Resetting one PX4 instance: put its gz world back to the initial pose
//! and stop exactly that instance's px4, so its supervisor starts a fresh
//! one whose estimator state resets with the vehicle.
//!
//! Only a supervised instance is reset. Every PX4 instance runs the same
//! binary, so the instance is told apart by its `-i` argument; a match on
//! the binary alone would stop every vehicle on the machine.

use std::path::Path;
use std::process::Command;

use crate::error::XtaskError;
use crate::output::print_line;

/// Resets PX4 instance `instance` of the session this checkout supervises.
///
/// # Errors
///
/// Returns [`XtaskError::Unsupervised`] when no live xtask supervisor runs
/// that instance (nothing would start the stopped px4 again), and a typed
/// failure when the world reset fails.
pub(super) fn reset_instance(
    repo_root: &Path,
    px4: &Path,
    world: &str,
    instance: u16,
) -> Result<(), XtaskError> {
    if !supervisor_alive(&crate::session::supervisor_marker(repo_root, instance)) {
        return Err(XtaskError::Unsupervised { instance });
    }
    print_line(&format!(
        "resetting world {world:?} of PX4 instance {instance}..."
    ));
    reset_world(world, instance)?;
    restart_instance(repo_root, px4, instance)
}

/// Stops exactly PX4 instance `instance` of the session this checkout
/// supervises, so its supervisor starts a fresh one.
///
/// # Errors
///
/// Returns [`XtaskError::Unsupervised`] when no live xtask supervisor runs
/// that instance: nothing would start the stopped px4 again.
pub(crate) fn restart_instance(
    repo_root: &Path,
    px4: &Path,
    instance: u16,
) -> Result<(), XtaskError> {
    if !supervisor_alive(&crate::session::supervisor_marker(repo_root, instance)) {
        return Err(XtaskError::Unsupervised { instance });
    }
    let binary = px4.join("build/px4_sitl_default/bin/px4");
    let pids = instance_pids(&binary.display().to_string(), instance);
    for pid in &pids {
        Command::new("kill").args(["-9", pid]).status().ok();
    }
    print_line(&format!(
        "stopped {} px4 process(es) of instance {instance}; the xtask supervisor restarts it",
        pids.len()
    ));
    Ok(())
}

/// Stops the `program` process of instance `instance` that holds UDP
/// `port`, for a flight controller whose command line does not name its
/// instance: the port it binds does. A peer that only talks to the port
/// (the session host) is another program and stays. Its supervisor
/// starts a fresh flight controller.
///
/// # Errors
///
/// Returns [`XtaskError::Unsupervised`] when no live xtask supervisor runs
/// that instance: nothing would start the stopped process again.
pub(crate) fn restart_port_holder(
    repo_root: &Path,
    instance: u16,
    port: u16,
    program: &str,
) -> Result<(), XtaskError> {
    if !supervisor_alive(&crate::session::supervisor_marker(repo_root, instance)) {
        return Err(XtaskError::Unsupervised { instance });
    }
    let pids = Command::new("lsof")
        .args(["-t", &format!("-iUDP:{port}")])
        .output()
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
        .into_iter()
        .filter(|pid| runs(pid, program))
        .collect::<Vec<_>>();
    for pid in &pids {
        Command::new("kill").args(["-9", pid]).status().ok();
    }
    print_line(&format!(
        "stopped {} flight controller process(es) of instance {instance}; the xtask \
         supervisor restarts it",
        pids.len()
    ));
    Ok(())
}

/// Whether process `pid` runs `program`.
fn runs(pid: &str, program: &str) -> bool {
    Command::new("ps")
        .args(["-o", "command=", "-p", pid])
        .output()
        .is_ok_and(|output| {
            String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .next()
                .is_some_and(|first| first.ends_with(program))
        })
}

/// Whether the pid in `marker` names a live process.
fn supervisor_alive(marker: &Path) -> bool {
    let Ok(pid) = std::fs::read_to_string(marker) else {
        return false;
    };
    Command::new("kill")
        .args(["-0", pid.trim()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn reset_world(world: &str, instance: u16) -> Result<(), XtaskError> {
    let mut command = Command::new("gz");
    command
        .args(["service", "-s", &format!("/world/{world}/control")])
        .args([
            "--reqtype",
            "gz.msgs.WorldControl",
            "--reptype",
            "gz.msgs.Boolean",
        ])
        .args(["--timeout", "3000", "--req", "reset: {all: true}"])
        .env("PATH", crate::backend::aviate_gz::search_path())
        .env("GZ_IP", "127.0.0.1")
        .env_remove("GZ_PARTITION");
    if let Some(partition) = crate::backend::gz::partition_for(instance, super::BACKEND_NAME) {
        command.env("GZ_PARTITION", partition);
    }
    let status = command.status().map_err(|source| XtaskError::Io {
        context: "running the gz world reset",
        source,
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(XtaskError::CommandFailed {
            context: "gz world reset",
            status: status.to_string(),
        })
    }
}

/// The pids of `binary` processes that run PX4 instance `instance`.
fn instance_pids(binary: &str, instance: u16) -> Vec<String> {
    let Ok(output) = Command::new("pgrep").args(["-f", binary]).output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .filter(|pid| {
            Command::new("ps")
                .args(["-o", "command=", "-p", pid])
                .output()
                .ok()
                .map(|line| String::from_utf8_lossy(&line.stdout).into_owned())
                .is_some_and(|line| {
                    line.split_whitespace().next() == Some(binary) && instance_of(&line) == instance
                })
        })
        .map(str::to_owned)
        .collect()
}

/// The instance a px4 command line runs: the value after `-i`, or 0 when
/// it has none (PX4's own default).
pub(super) fn instance_of(command_line: &str) -> u16 {
    let mut args = command_line.split_whitespace();
    while let Some(arg) = args.next() {
        if arg == "-i" {
            return args
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
        }
    }
    0
}

#[cfg(test)]
mod tests;
