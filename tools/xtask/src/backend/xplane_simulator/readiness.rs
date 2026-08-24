//! X-Plane process and weather readiness gates.

use std::path::Path;

use super::{Airframe, send_xplane_command};
use crate::error::XtaskError;
use crate::output::print_line;
use crate::readiness::stage_log;

const WEATHER_READY_TIMEOUT_S: u64 = 420;

/// Test whether an X-Plane simulator process is running on this machine.
pub(in crate::backend) fn xplane_running_blocking() -> Result<bool, XtaskError> {
    xplane_running_with_program_blocking(Path::new("pgrep"))
}

pub(in crate::backend) fn xplane_running_with_program_blocking(
    program: &Path,
) -> Result<bool, XtaskError> {
    let status = std::process::Command::new(program)
        .args(["-f", "X-Plane.app/Contents/MacOS/X-Plane"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|source| XtaskError::Io {
            context: "probing for a running X-Plane process",
            source,
        })?;
    classify_xplane_probe_status(status)
}

pub(in crate::backend) fn classify_xplane_probe_status(
    status: std::process::ExitStatus,
) -> Result<bool, XtaskError> {
    match status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(XtaskError::CommandFailed {
            context: "X-Plane process probe",
            status: status.to_string(),
        }),
    }
}

pub(in crate::backend) fn run_xplane_prepare_sequence<WarmProof, Launch, ColdProof, Connect>(
    simulator_running: bool,
    warm_proof: WarmProof,
    launch: Launch,
    cold_proof: ColdProof,
    connect: Connect,
) -> Result<(), XtaskError>
where
    WarmProof: FnOnce() -> Result<(), XtaskError>,
    Launch: FnOnce() -> Result<(), XtaskError>,
    ColdProof: FnOnce() -> Result<(), XtaskError>,
    Connect: FnOnce(),
{
    if simulator_running {
        warm_proof()?;
        connect();
        return Ok(());
    }
    launch()?;
    cold_proof()
}

/// Prepare one running or cold X-Plane process before the FC can start.
pub(in crate::backend) fn prepare_xplane_runtime_blocking(
    repo_root: &Path,
    root: &Path,
    airframe: &Airframe,
    log_dir: &Path,
    simulator_running: bool,
) -> Result<(), XtaskError> {
    run_xplane_prepare_sequence(
        simulator_running,
        || verify_weather_plugin_blocking(repo_root),
        || launch_xplane_blocking(root, airframe, log_dir),
        || wait_for_weather_plugin_blocking(repo_root),
        || send_xplane_command("px4xplane/connect"),
    )
}

/// Prove an acknowledged calm transaction before a controller can start.
pub(in crate::backend) fn verify_weather_plugin_blocking(
    repo_root: &Path,
) -> Result<(), XtaskError> {
    let status = weather_clear_status_blocking(repo_root)?;
    if status.success() {
        return Ok(());
    }
    Err(XtaskError::SimulatorCapability {
        capability: "PilotageWeather clear transaction",
        detail: status.to_string(),
    })
}

fn wait_for_weather_plugin_blocking(repo_root: &Path) -> Result<(), XtaskError> {
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(WEATHER_READY_TIMEOUT_S);
    let mut last_status = String::from("not started");
    while std::time::Instant::now() < deadline {
        let status = weather_clear_status_blocking(repo_root)?;
        if status.success() {
            return Ok(());
        }
        last_status = status.to_string();
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    Err(XtaskError::SimulatorCapability {
        capability: "PilotageWeather clear transaction",
        detail: format!(
            "did not succeed within {WEATHER_READY_TIMEOUT_S} s; last status {last_status}"
        ),
    })
}

fn weather_clear_status_blocking(repo_root: &Path) -> Result<std::process::ExitStatus, XtaskError> {
    std::process::Command::new("python3")
        .arg(repo_root.join("scripts/xplane_weather_clear.py"))
        .current_dir(repo_root)
        .status()
        .map_err(|source| XtaskError::Io {
            context: "running the X-Plane weather clear proof",
            source,
        })
}

fn launch_xplane_blocking(
    root: &Path,
    airframe: &Airframe,
    log_dir: &Path,
) -> Result<(), XtaskError> {
    print_line("starting X-Plane (a cold boot takes minutes)...");
    // The reset process captures one home position for each X-Plane process.
    if let Some(home) = std::env::var_os("HOME") {
        std::fs::remove_file(std::path::PathBuf::from(home).join(".pilotage/xplane-home.json"))
            .ok();
    }
    let log = std::fs::File::create(stage_log(log_dir, "xplane"))
        .ok()
        .map_or(std::process::Stdio::null(), std::process::Stdio::from);
    std::process::Command::new(root.join("X-Plane.app/Contents/MacOS/X-Plane"))
        .arg("--window=1400x900")
        .arg("--pref:_show_qfl_on_start=0")
        .current_dir(root)
        .env("PILOTAGE_XPLANE_ACF", airframe.acf_path)
        .env("PILOTAGE_XPLANE_CONNECT", "1")
        // A cold start can exceed the bridge connection time limit.
        .env("PX4XPLANE_CONNECT_TIMEOUT_S", "0")
        .stdout(log)
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|source| XtaskError::Spawn {
            name: "X-Plane",
            source,
        })
}
