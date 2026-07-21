//! The PX4 + Gazebo SITL backend, orchestrated by this launcher rather
//! than PX4's own scripts: xtask starts the gz server with PX4's world,
//! sensor systems, and plugins, then runs the px4 binary in standalone
//! mode so it attaches to that server and spawns the x500 model. The
//! adapter side is FDM-agnostic — swapping gz for JSBSim or FlightGear
//! is a new backend planning different stages, not a new adapter.
//!
//! Gimbal link-loss acceptance — validating PX4's INDEPENDENT failsafe
//! (MANUAL): the host's failsafe (`queue_link_loss_stop`) is a best-effort
//! queued zero-rate; its declared independent backstop is PX4's own
//! gimbal-manager setpoint-timeout, which zeroes a nonzero angular rate after
//! ~2 s (`src/modules/gimbal/output.cpp` `check_and_handle_setpoint_timeout`,
//! `timestamp_last_update + 2_s`). A plain flight does NOT validate the backstop
//! — the host's own stop would halt the gimbal regardless. The DISCRIMINATING
//! procedure DROPS the host's stop (fault injection) so PX4's timeout is the
//! SOLE mechanism: launch with `PILOTAGE_PX4_DROP_GIMBAL_STOP=1 cargo xtask sim
//! px4-gz`, slew the gimbal at a sustained nonzero rate, sever the control link
//! mid-slew, and confirm the gimbal KEEPS slewing (the host sent no stop) then
//! stops ~2 s later — that stop is PX4's own timeout, not Pilotage's.
//!
//! Validated PX4 SHA: `6120aa53df874021639e2413a4cdecf8df8e355a`
//! (`v1.18.0-beta1-110-g6120aa53df`). Status: fault-injection exercised; PX4
//! outcome pending. On 2026-07-21 this backend was flown with the fault
//! injection: PX4 accepted Pilotage's primary-gimbal-control claim (`[gimbal]
//! Configured primary gimbal control ... to 255/190`), and on holder disconnect
//! the session host logged, reproducibly, `holder lost; engaging link-loss
//! policy scope="vehicle.gimbal"` followed by `gimbal link-loss stop DROPPED
//! (fault injection); relying on PX4's own timeout`. That trace proves only the
//! Pilotage half — the host provably sent NO stop. The PX4 half — the gimbal
//! keeping its rate and PX4 zeroing it ~2 s later — is code-verified against
//! `output.cpp` above but NOT yet observed on the wire; #168 tracks capturing
//! the gz/MAVLink rate-vs-time trace that would close it. No automated
//! PX4-in-the-loop test runs in CI.

use std::path::{Path, PathBuf};

use super::{SessionContext, SimBackend, Stage};
use crate::cli::Profile;
use crate::error::XtaskError;
use crate::output::print_line;
use crate::process::ProcessSpec;
use crate::readiness::{Readiness, stage_log};

/// The gz world PX4's x500 model spawns into (PX4 ships `default.sdf`;
/// its `<world name>` is `default` — also what the reset script targets).
const WORLD_NAME: &str = "default";
/// PX4 airframe autostart id for the gz x500 with the CGO3 gimbal
/// (4019_gz_x500_gimbal): MNT_MODE_IN/OUT = MAVLink Gimbal Protocol v2,
/// which the PX4 adapter's `vehicle.gimbal` scope drives (GIM-04).
const SYS_AUTOSTART: &str = "4019";
/// The matching PX4 model selector for the airframe above.
const SIM_MODEL: &str = "gz_x500_gimbal";

/// The PX4 + Gazebo SITL backend.
#[derive(Debug)]
pub struct Px4Gz;

impl SimBackend for Px4Gz {
    fn name(&self) -> &'static str {
        "px4-gz"
    }

    fn host_adapter(&self) -> &'static str {
        "px4"
    }

    fn host_env(&self, _ctx: &SessionContext) -> Vec<(String, String)> {
        vec![
            ("GZ_IP".to_owned(), "127.0.0.1".to_owned()),
            ("PILOTAGE_PX4_PROFILE".to_owned(), "simulation".to_owned()),
            // This backend boots the gz_x500_gimbal airframe (4019), so
            // the adapter advertises the vehicle.gimbal scope.
            ("PILOTAGE_PX4_GIMBAL".to_owned(), "1".to_owned()),
        ]
    }

    fn plan(&self, ctx: &SessionContext) -> Result<Vec<Stage>, XtaskError> {
        // The PX4 adapter implements only the simulation profile. Keep
        // the launcher boundary aligned with the host boundary so no
        // unsupported session reaches process startup.
        if ctx.profile != Profile::Simulation {
            return Err(XtaskError::Usage {
                message: format!(
                    "the px4-gz backend supports only --profile simulation (got {:?})",
                    ctx.profile
                ),
            });
        }
        plan_with_px4_dir(ctx, &px4_dir(&ctx.repo_root))
    }

    fn prepare(&self, ctx: &SessionContext) -> Result<(), XtaskError> {
        ensure_camera_bridge(&ctx.repo_root);
        Ok(())
    }

    fn stale_process_patterns(&self) -> Vec<&'static str> {
        vec!["gz sim", "bin/px4"]
    }

    fn reset(&self, repo_root: &Path) -> Result<(), XtaskError> {
        let script = repo_root.join("scripts/reset-px4-sim.sh");
        let status = std::process::Command::new("bash")
            .arg(&script)
            .arg(WORLD_NAME)
            .env("PATH", super::aviate_gz::search_path())
            .env("PX4_DIR", px4_dir(repo_root))
            .status()
            .map_err(|source| XtaskError::Io {
                context: "running the PX4 reset script",
                source,
            })?;
        if status.success() {
            Ok(())
        } else {
            Err(XtaskError::CommandFailed {
                context: "PX4 reset script",
                status: status.to_string(),
            })
        }
    }
}

/// The gitignored C++ camera sidecar the px4-gz FPV/chase video needs.
fn camera_bridge_bin(repo_root: &Path) -> PathBuf {
    repo_root.join("adapters/gazebo/bridge/build/pilotage-gz-bridge")
}

/// Best-effort build of the camera sidecar so a fresh checkout shows video
/// out of the box. It is DELIBERATELY non-fatal: the px4 adapter's camera
/// path degrades to no-video when the binary is absent, so a missing C++
/// toolchain (gz-transport, protoc) must not block the flight — it only
/// costs the camera. A present binary is left untouched.
fn ensure_camera_bridge(repo_root: &Path) {
    if camera_bridge_bin(repo_root).is_file() {
        return;
    }
    print_line("building the gz camera sidecar (first run)...");
    let built = std::process::Command::new("bash")
        .arg(repo_root.join("scripts/build-gz-bridge.sh"))
        .current_dir(repo_root)
        .status();
    match built {
        Ok(status) if status.success() => print_line("gz camera sidecar built"),
        Ok(_) | Err(_) => print_line(
            "gz camera sidecar unavailable (see build-gz-bridge output); \
             continuing without video",
        ),
    }
}

/// Where the PX4-Autopilot checkout lives: `PX4_DIR`, else
/// `../PX4-Autopilot` next to this repository. A directory convention,
/// never a source dependency.
fn px4_dir(repo_root: &Path) -> PathBuf {
    let dir = std::env::var_os("PX4_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root.join("../PX4-Autopilot"));
    // Canonical, or the reset script's exact-path process match never
    // sees the spawned binary (a literal `..` in the command line).
    dir.canonicalize().unwrap_or(dir)
}

/// The testable core of [`Px4Gz::plan`]: validates every artifact with
/// an actionable hint, then assembles the gz and PX4 stages.
fn plan_with_px4_dir(ctx: &SessionContext, px4: &Path) -> Result<Vec<Stage>, XtaskError> {
    if !px4.is_dir() {
        return Err(XtaskError::MissingArtifact {
            what: "PX4-Autopilot checkout",
            path: px4.to_path_buf(),
            hint: "clone PX4-Autopilot next to this repository or set PX4_DIR",
        });
    }
    let binary = px4.join("build/px4_sitl_default/bin/px4");
    if !binary.is_file() {
        return Err(XtaskError::MissingArtifact {
            what: "PX4 SITL binary",
            path: binary,
            hint: "build it: make px4_sitl in the PX4-Autopilot checkout",
        });
    }
    let world = ctx.repo_root.join("sim/worlds/px4_flightdeck.sdf");
    if !world.is_file() {
        return Err(XtaskError::MissingArtifact {
            what: "px4 flight-deck world",
            path: world,
            hint: "run from the Pilotage repository root",
        });
    }
    let server_config = px4.join("Tools/simulation/gz/server.config");
    if !server_config.is_file() {
        return Err(XtaskError::MissingArtifact {
            what: "PX4 gz server config",
            path: server_config,
            hint: "the PX4 checkout is missing Tools/simulation/gz/server.config",
        });
    }
    let path = super::aviate_gz::search_path();
    Ok(vec![
        gz_stage(ctx, px4, &path, &world, &server_config),
        px4_stage(ctx, px4, &path, &binary)?,
    ])
}

fn gz_stage(
    ctx: &SessionContext,
    px4: &Path,
    path: &str,
    world: &Path,
    server_config: &Path,
) -> Stage {
    // PX4's worlds carry no inline system plugins; the sensor systems
    // (imu, magnetometer, navsat, air pressure) come from the server
    // config, and PX4's own gz plugins from the build tree.
    let gz_env = vec![
        ("PATH".to_owned(), path.to_owned()),
        ("GZ_IP".to_owned(), "127.0.0.1".to_owned()),
        (
            "GZ_SIM_SERVER_CONFIG_PATH".to_owned(),
            server_config.display().to_string(),
        ),
        (
            "GZ_SIM_SYSTEM_PLUGIN_PATH".to_owned(),
            px4.join("build/px4_sitl_default/src/modules/simulation/gz_plugins")
                .display()
                .to_string(),
        ),
        (
            "GZ_SIM_RESOURCE_PATH".to_owned(),
            format!(
                "{}:{}:{}:{}",
                ctx.repo_root.join("sim/worlds").display(),
                ctx.repo_root.join("sim/models").display(),
                px4.join("Tools/simulation/gz/worlds").display(),
                px4.join("Tools/simulation/gz/models").display()
            ),
        ),
    ];
    Stage {
        spec: ProcessSpec {
            name: "gazebo",
            program: "gz".to_owned(),
            args: vec![
                "sim".to_owned(),
                "-s".to_owned(),
                "-r".to_owned(),
                "--headless-rendering".to_owned(),
                world.display().to_string(),
            ],
            cwd: None,
            env: gz_env.clone(),
            remove_env: vec!["DISPLAY"],
            log_path: stage_log(&ctx.log_dir, "gazebo"),
        },
        readiness: Readiness::CommandOutput {
            program: "gz".to_owned(),
            args: vec!["topic".to_owned(), "-l".to_owned()],
            env: gz_env,
            needle: WORLD_NAME,
            timeout_s: 60,
        },
    }
}

fn px4_stage(
    ctx: &SessionContext,
    px4: &Path,
    path: &str,
    binary: &Path,
) -> Result<Stage, XtaskError> {
    // px4 resolves its startup script relative to the working directory
    // and litters it with eeprom/dataman state; a dedicated rootfs dir
    // inside the build tree keeps that contained.
    let rootfs = px4.join("build/px4_sitl_default/rootfs");
    std::fs::create_dir_all(&rootfs).map_err(|source| XtaskError::Io {
        context: "creating the PX4 rootfs directory",
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
                ("PATH".to_owned(), path.to_owned()),
                ("GZ_IP".to_owned(), "127.0.0.1".to_owned()),
                // Standalone: xtask owns the gz server; px4 attaches to
                // the statically included model (which carries the
                // Pilotage camera rig) instead of spawning its own.
                ("PX4_GZ_STANDALONE".to_owned(), "1".to_owned()),
                ("PX4_SYS_AUTOSTART".to_owned(), SYS_AUTOSTART.to_owned()),
                ("PX4_SIM_MODEL".to_owned(), SIM_MODEL.to_owned()),
                ("PX4_GZ_MODEL_NAME".to_owned(), "x500_0".to_owned()),
                ("PX4_GZ_WORLD".to_owned(), WORLD_NAME.to_owned()),
            ],
            remove_env: vec![],
            log_path: stage_log(&ctx.log_dir, "flight-controller"),
        },
        // "Ready for takeoff" waits on a GCS heartbeat, which only the
        // session host provides — a later stage. Boot completion is the
        // honest FC-stage signal.
        readiness: Readiness::LogContains {
            needle: "Startup script returned successfully",
            timeout_s: 60,
        },
    })
}

#[cfg(test)]
mod tests;
