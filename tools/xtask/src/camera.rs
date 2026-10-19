//! The world-model camera stage any backend can add (`--camera maplibre`).
//!
//! The session host binds a loopback sim camera port and turns off the
//! adapter's own camera, and the `sim/maplibre-camera` producer dials that
//! port and renders the vehicle's cameras from the host's simulator truth.
//! The stage is independent of the backend: a vehicle whose adapter
//! publishes no simulator truth gets an attached producer that renders
//! nothing.

use std::path::Path;
use std::process::Command;

use crate::backend::{SessionContext, Stage};
use crate::error::XtaskError;
use crate::process::ProcessSpec;
use crate::readiness::{Readiness, stage_log};

/// The sim camera port of instance 0; instance `k` uses this plus `k`, so
/// sessions on different instances never share one.
pub const SIM_CAMERA_PORT_BASE: u16 = 46_000;
/// The producer's crate, built on its own (it is outside the workspace).
const CAMERA_MANIFEST: &str = "sim/maplibre-camera/Cargo.toml";
/// The producer binary the build leaves.
const CAMERA_BINARY: &str = "sim/maplibre-camera/target/release/pilotage-maplibre-camera";
/// The line the producer logs once its renderers and link are up.
const READY_LINE: &str = "maplibre camera producer up";
/// How long the producer may take to cache its site's tiles (it gives up on
/// the rest after four minutes) and build its GPU renderers, s.
const READY_TIMEOUT_S: u64 = 330;

/// The sim camera port of `ctx`'s instance.
///
/// # Errors
///
/// [`XtaskError::Usage`] when the instance puts the port past 65535.
pub fn port(ctx: &SessionContext) -> Result<u16, XtaskError> {
    SIM_CAMERA_PORT_BASE
        .checked_add(ctx.instance)
        .ok_or_else(|| XtaskError::Usage {
            message: format!(
                "instance {} has no sim camera port (base {SIM_CAMERA_PORT_BASE})",
                ctx.instance
            ),
        })
}

/// The host environment of a session with the camera stage: the sim camera
/// port, and every adapter's own camera off, since the host refuses two
/// producers for one session.
#[must_use]
pub fn host_env(port: u16) -> Vec<(String, String)> {
    vec![
        ("PILOTAGE_SIM_CAMERA_PORT".to_owned(), port.to_string()),
        ("PILOTAGE_PX4_CAMERA".to_owned(), "off".to_owned()),
        ("PILOTAGE_AVIATE_CAMERA".to_owned(), "off".to_owned()),
    ]
}

/// The producer stage, started after the host so its port is bound. With
/// the vehicle's spawn point known, the producer caches the tiles around it
/// before it reports ready, so the cameras open on imagery.
///
/// # Errors
///
/// [`XtaskError::Usage`] when the instance has no sim camera port.
pub fn stage(ctx: &SessionContext, spawn: Option<(f64, f64)>) -> Result<Stage, XtaskError> {
    let port = port(ctx)?;
    let mut env = Vec::new();
    if std::env::var_os("RUST_LOG").is_none() {
        env.push(("RUST_LOG".to_owned(), "info,wgpu=warn,naga=warn".to_owned()));
    }
    let mut args = vec![
        "--host-port".to_owned(),
        port.to_string(),
        "--calibration-dir".to_owned(),
        ctx.log_dir.join("calibrations").display().to_string(),
    ];
    if let Some((latitude, longitude)) = spawn {
        args.push("--prewarm".to_owned());
        args.push(format!("{latitude},{longitude}"));
    }
    Ok(Stage {
        spec: ProcessSpec {
            name: "maplibre-camera",
            program: ctx.repo_root.join(CAMERA_BINARY).display().to_string(),
            args,
            cwd: Some(ctx.repo_root.clone()),
            env,
            remove_env: vec![],
            log_path: stage_log(&ctx.log_dir, "maplibre-camera"),
        },
        readiness: Readiness::LogContains {
            needle: READY_LINE,
            timeout_s: READY_TIMEOUT_S,
        },
    })
}

/// Builds the producer in release.
///
/// # Errors
///
/// [`XtaskError::Io`] when cargo cannot run and
/// [`XtaskError::CommandFailed`] when the build fails.
pub fn build(repo_root: &Path) -> Result<(), XtaskError> {
    let status = Command::new("cargo")
        .args(["build", "--release", "--manifest-path", CAMERA_MANIFEST])
        .current_dir(repo_root)
        .status()
        .map_err(|source| XtaskError::Io {
            context: "building the maplibre camera",
            source,
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(XtaskError::CommandFailed {
            context: "cargo build --release --manifest-path sim/maplibre-camera/Cargo.toml",
            status: status.to_string(),
        })
    }
}

/// Parses `--camera`: `maplibre` adds the world-model camera stage, `none`
/// leaves the backend's own video.
///
/// # Errors
///
/// [`XtaskError::Usage`] for any other camera.
pub fn parse_choice(value: &str) -> Result<bool, XtaskError> {
    match value {
        "maplibre" => Ok(true),
        "none" => Ok(false),
        other => Err(XtaskError::Usage {
            message: format!("unknown camera {other:?} (expected maplibre or none)"),
        }),
    }
}

#[cfg(test)]
mod tests;
