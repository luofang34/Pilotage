//! The PX4 + X-Plane SITL backend: X-Plane runs as an external,
//! operator-owned desktop application (like a physical bench device),
//! while the px4xplane X-Plane plugin bridges the standard MAVLink HIL
//! stream (`HIL_SENSOR`/`HIL_GPS` in, `HIL_ACTUATOR_CONTROLS` out) to
//! PX4 over TCP 4560. PX4 selects that path with `PX4_SIMULATOR=xplane`
//! and dials the plugin, so stage order is forgiving: PX4 retries until
//! the plugin listens. The adapter side is untouched — the PX4 adapter
//! is FDM-agnostic, so this backend only plans different stages.
//!
//! The launcher never manages the X-Plane process: it is not a stage,
//! never a stale-process pattern, and reset never kills it. When
//! X-Plane is not running, `prepare` starts it detached with the
//! PilotageAutoFlight environment so the flight loads and the SITL
//! listener arms without operator input; when it is already running,
//! `prepare` arms the listener with an X-Plane UDP `CMND` datagram.

use std::path::Path;

use super::xplane_simulator::{
    Airframe, ensure_xplane_plugins_blocking, prepare_xplane_runtime_blocking, selected_airframe,
    set_active_config_name, set_ground_sensor_contract, validate_xplane_install,
    verify_loaded_aircraft, xplane_root, xplane_running_blocking,
};
use super::{SessionContext, SimBackend, Stage};
use crate::cli::Profile;
use crate::error::XtaskError;
use crate::output::print_line;
use crate::process::ProcessSpec;
use crate::readiness::{Readiness, stage_log};

/// The PX4 + X-Plane SITL backend.
#[derive(Debug)]
pub struct Px4XPlane;

impl SimBackend for Px4XPlane {
    fn name(&self) -> &'static str {
        "px4-xplane"
    }

    fn host_adapter(&self) -> &'static str {
        "px4"
    }

    fn host_env(&self, ctx: &SessionContext) -> Vec<(String, String)> {
        vec![
            ("PILOTAGE_PX4_PROFILE".to_owned(), "simulation".to_owned()),
            // No gimbal device exists in the X-Plane HIL bridge; the
            // scope must not be advertised. X-Plane exposes no camera
            // topic either, so the vehicle view comes from the
            // in-simulator camera plugin, which dials the host.
            ("PILOTAGE_PX4_CAMERA".to_owned(), "xplane-plugin".to_owned()),
            (
                "PILOTAGE_RESET_CMD".to_owned(),
                ctx.repo_root
                    .join("scripts/reset-xplane-sim.sh")
                    .display()
                    .to_string(),
            ),
        ]
    }

    fn plan(&self, ctx: &SessionContext) -> Result<Vec<Stage>, XtaskError> {
        if ctx.profile != Profile::Simulation {
            return Err(XtaskError::Usage {
                message: format!(
                    "the px4-xplane backend supports only --profile simulation (got {:?})",
                    ctx.profile
                ),
            });
        }
        let airframe = selected_airframe()?;
        let root = xplane_root()?;
        validate_xplane_install(&root, airframe)?;
        let px4 = super::px4_gz::px4_dir(&ctx.repo_root);
        let binary = px4.join("build/px4_sitl_default/bin/px4");
        if !binary.is_file() {
            return Err(XtaskError::MissingArtifact {
                what: "PX4 SITL binary",
                path: binary,
                hint: "build it: make px4_sitl in the PX4-Autopilot checkout",
            });
        }
        Ok(vec![px4_stage(ctx, &px4, &binary, airframe)?])
    }

    fn prepare(&self, ctx: &SessionContext) -> Result<(), XtaskError> {
        ensure_xplane_plugins_blocking(&ctx.repo_root, xplane_running_blocking()?)?;
        let Ok(airframe) = selected_airframe() else {
            // plan() re-runs the same resolution and reports the error.
            return Ok(());
        };
        let Ok(root) = xplane_root() else {
            return Ok(());
        };
        let simulator_running = xplane_running_blocking()?;
        // A launcher that starts X-Plane also chooses the aircraft. One
        // that finds it already running chooses only the bridge
        // configuration, and a configuration that names another aircraft
        // is answered and then dropped, with nothing on either side
        // saying why.
        if simulator_running {
            verify_loaded_aircraft(&root, airframe)?;
        }
        set_active_config_name(&root, airframe);
        // PX4's EKF wants the bridge's ground-stationary contract; the
        // aviate lane turns it off, so this lane must turn it back on.
        set_ground_sensor_contract(&root, true);
        prepare_xplane_runtime_blocking(
            &ctx.repo_root,
            &root,
            airframe,
            &ctx.log_dir,
            simulator_running,
        )?;
        if simulator_running {
            print_line("X-Plane weather is ready and the SITL listener is armed");
            print_line("if PX4 cannot connect: Plugins > PX4 X-Plane > Connect to SITL");
        }
        Ok(())
    }

    fn stale_process_patterns(&self) -> Vec<&'static str> {
        // X-Plane is deliberately absent: the launcher never owns it.
        vec!["bin/px4"]
    }

    fn reset(&self, repo_root: &Path) -> Result<(), XtaskError> {
        let script = repo_root.join("scripts/reset-xplane-sim.sh");
        let status = std::process::Command::new("bash")
            .arg(&script)
            .env("PX4_DIR", super::px4_gz::px4_dir(repo_root))
            .status()
            .map_err(|source| XtaskError::Io {
                context: "running the X-Plane reset script",
                source,
            })?;
        if status.success() {
            Ok(())
        } else {
            Err(XtaskError::CommandFailed {
                context: "X-Plane reset script",
                status: status.to_string(),
            })
        }
    }
}

/// The PX4 SITL stage: `PX4_SIMULATOR=xplane` routes the startup script
/// to `simulator_mavlink`, which dials the plugin on TCP 4560 and
/// retries until it answers. The readiness needle appears only after
/// that link is up, so the deadline covers an X-Plane cold boot.
fn px4_stage(
    ctx: &SessionContext,
    px4: &Path,
    binary: &Path,
    airframe: &Airframe,
) -> Result<Stage, XtaskError> {
    // A rootfs of its own: SITL parameters persist in the rootfs, and a
    // stale parameters.bson from another simulator's airframe would
    // override this airframe's defaults.
    let rootfs = px4.join("build/px4_sitl_default/rootfs-xplane");
    std::fs::create_dir_all(&rootfs).map_err(|source| XtaskError::Io {
        context: "creating the PX4 X-Plane rootfs directory",
        source,
    })?;
    Ok(Stage {
        spec: ProcessSpec {
            name: "flight-controller",
            program: binary.display().to_string(),
            args: vec![
                px4.join("build/px4_sitl_default/etc").display().to_string(),
                "-s".to_owned(),
                "etc/init.d-posix/rcS".to_owned(),
                "-d".to_owned(),
            ],
            cwd: Some(rootfs),
            env: vec![
                ("PX4_SIMULATOR".to_owned(), "xplane".to_owned()),
                ("PX4_SIM_HOSTNAME".to_owned(), "127.0.0.1".to_owned()),
                (
                    "PX4_SYS_AUTOSTART".to_owned(),
                    airframe.sys_autostart.to_owned(),
                ),
            ],
            remove_env: vec![],
            log_path: stage_log(&ctx.log_dir, "flight-controller"),
        },
        readiness: Readiness::LogContains {
            needle: "Startup script returned successfully",
            timeout_s: 420,
        },
    })
}

#[cfg(test)]
mod tests;
