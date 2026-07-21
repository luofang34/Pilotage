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
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use std::path::PathBuf;

    use super::{Px4Gz, plan_with_px4_dir};
    use crate::backend::{SessionContext, SimBackend};
    use crate::cli::Profile;
    use crate::error::XtaskError;

    fn context(repo_root: PathBuf) -> SessionContext {
        SessionContext {
            repo_root,
            host_port: 4433,
            viewer_port: 8080,
            profile: Profile::Simulation,
            log_dir: std::env::temp_dir(),
        }
    }

    fn scaffold(tag: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("px4-gz-plan-{tag}-{}", std::process::id()));
        let px4 = root.join("PX4-Autopilot");
        std::fs::create_dir_all(&px4).expect("scaffold");
        (root, px4)
    }

    #[test]
    fn plan_refuses_physical_and_oracle_only_profiles() {
        let backend = Px4Gz;
        for profile in [Profile::Physical, Profile::OracleOnly] {
            let mut ctx = context(PathBuf::from("unused-for-profile-refusal"));
            ctx.profile = profile;
            let refusal = backend.plan(&ctx);
            assert!(
                matches!(refusal, Err(XtaskError::Usage { .. })),
                "{profile:?} must be refused, got {refusal:?}"
            );
        }
    }

    #[test]
    fn host_environment_declares_the_px4_simulation_profile() {
        let backend = Px4Gz;
        let ctx = context(PathBuf::from("unused-for-host-environment"));
        assert!(
            backend
                .host_env(&ctx)
                .iter()
                .any(|(key, value)| { key == "PILOTAGE_PX4_PROFILE" && value == "simulation" })
        );
    }

    #[test]
    fn host_environment_enables_the_gimbal_capability() {
        // The gz_x500_gimbal airframe carries a gimbal; the host must
        // advertise the scope, and no other FC backend sets this flag.
        let backend = Px4Gz;
        let ctx = context(PathBuf::from("unused-for-host-environment"));
        assert!(
            backend
                .host_env(&ctx)
                .iter()
                .any(|(key, value)| key == "PILOTAGE_PX4_GIMBAL" && value == "1")
        );
    }

    #[test]
    fn missing_artifacts_fail_with_actionable_hints() {
        let (root, px4) = scaffold("missing");
        let ctx = context(root.clone());

        let refusal = plan_with_px4_dir(&ctx, &root.join("absent"));
        assert!(matches!(
            refusal,
            Err(XtaskError::MissingArtifact {
                what: "PX4-Autopilot checkout",
                ..
            })
        ));

        let refusal = plan_with_px4_dir(&ctx, &px4);
        assert!(matches!(
            refusal,
            Err(XtaskError::MissingArtifact {
                what: "PX4 SITL binary",
                ..
            })
        ));
        std::fs::remove_dir_all(&root).ok();
    }

    // The two FC families keep separate worlds (their physics steps,
    // vehicle models, and FC glue genuinely differ), but the flight
    // deck must LOOK the same from the cameras: one green field, one
    // sun, one rig. This pins the shared appearance so the two files
    // cannot drift apart silently.
    #[test]
    fn both_flight_deck_worlds_share_the_same_look() {
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("repo root");
        let aviate = std::fs::read_to_string(repo_root.join("sim/worlds/x500_flightdeck.sdf"))
            .expect("aviate world");
        let px4 = std::fs::read_to_string(repo_root.join("sim/worlds/px4_flightdeck.sdf"))
            .expect("px4 world");
        for invariant in [
            "<uri>model://flightdeck_scenery</uri>",
            "<direction>-0.5 0.1 -0.9</direction>",
            "<magnetic_field>",
            "<model name=\"x500_camera_rig\">",
            "<topic>camera</topic>",
            "<topic>chase_camera</topic>",
        ] {
            assert!(aviate.contains(invariant), "aviate world lost {invariant}");
            assert!(px4.contains(invariant), "px4 world lost {invariant}");
        }
        // Neither world may carry its own ground: the field lives in
        // the ONE shared scenery model (green, 500 m) so future props
        // appear for every FC family at once.
        assert!(!aviate.contains("ground_plane") && !px4.contains("ground_plane"));
        let scenery =
            std::fs::read_to_string(repo_root.join("sim/models/flightdeck_scenery/model.sdf"))
                .expect("scenery model");
        assert!(scenery.contains("<ambient>0.3 0.5 0.3 1</ambient>"));
        assert!(scenery.contains("<size>500 500</size>"));
        // The default sky is part of the look: neither world may
        // override the scene with a gray background.
        assert!(!aviate.contains("<scene>") && !px4.contains("<scene>"));
    }

    #[test]
    fn a_complete_checkout_plans_gz_then_px4_standalone() {
        let (root, px4) = scaffold("complete");
        for file in [
            "build/px4_sitl_default/bin/px4",
            "Tools/simulation/gz/server.config",
        ] {
            let path = px4.join(file);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
            std::fs::write(&path, b"x").expect("file");
        }
        let world = root.join("sim/worlds/px4_flightdeck.sdf");
        std::fs::create_dir_all(world.parent().expect("parent")).expect("dirs");
        std::fs::write(&world, b"x").expect("world");
        let ctx = context(root.clone());
        let stages = plan_with_px4_dir(&ctx, &px4).expect("plan");
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0].spec.name, "gazebo");
        assert_eq!(stages[1].spec.name, "flight-controller");
        let fc_env = &stages[1].spec.env;
        assert!(
            fc_env
                .iter()
                .any(|(k, v)| k == "PX4_GZ_STANDALONE" && v == "1"),
            "px4 must attach to the xtask-owned gz server, not spawn its own"
        );
        assert!(
            fc_env
                .iter()
                .any(|(k, v)| k == "PX4_GZ_MODEL_NAME" && v == "x500_0"),
            "px4 must attach to the world's rig-bearing model, not spawn one"
        );
        assert!(
            stages[0]
                .spec
                .env
                .iter()
                .any(|(k, _)| k == "GZ_SIM_SERVER_CONFIG_PATH"),
            "gz must load PX4's sensor systems via the server config"
        );
        assert!(
            fc_env
                .iter()
                .any(|(k, v)| k == "PX4_SYS_AUTOSTART" && v == "4019")
                && fc_env
                    .iter()
                    .any(|(k, v)| k == "PX4_SIM_MODEL" && v == "gz_x500_gimbal"),
            "the FC must boot the gimbal airframe (GIM-04): 4019/gz_x500_gimbal"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    // The airframe env above and the statically included world model
    // must agree: PX4's GZGimbal bridge publishes joint commands under
    // `/model/<PX4_GZ_MODEL_NAME>/…`, so a world still carrying the
    // plain x500 would boot cleanly and then ignore every gimbal
    // demand silently.
    #[test]
    fn the_px4_world_carries_the_gimbal_model_the_airframe_expects() {
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("repo root");
        let world = std::fs::read_to_string(repo_root.join("sim/worlds/px4_flightdeck.sdf"))
            .expect("px4 world");
        assert!(
            world.contains("<uri>model://x500_gimbal</uri>"),
            "the px4 world must include the gimbal-bearing vehicle"
        );
        assert!(
            world.contains("<name>x500_0</name>"),
            "the included vehicle must keep the name PX4_GZ_MODEL_NAME attaches to"
        );
    }
}
