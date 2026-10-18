//! The Aviate + Gazebo SITL backend: the proven flight-deck runbook as
//! code — headless gz with the Aviate plugin, then the SITL FC over the
//! versioned shm block, each gated on its own readiness signal.

use std::path::Path;

use super::{SessionContext, SimBackend, Stage};
use crate::conflicts::{Claim, Remedy};
use crate::error::XtaskError;
use crate::process::ProcessSpec;
use crate::readiness::{Readiness, stage_log};

/// The world this backend launches; its `<world name>` is `aviate_sitl`
/// (also what the reset script targets).
const WORLD: &str = "sim/worlds/x500_flightdeck.sdf";
const WORLD_NAME: &str = "aviate_sitl";
/// This backend's `--fc` name; it also names its gz partitions.
const BACKEND_NAME: &str = "aviate-gz";
/// Where instance 0's FC streams telemetry and its host's link listens:
/// the default of both ends (the FC's `AviateApp.toml`, the host's link).
const DEFAULT_TELEMETRY_PORT: u16 = 14_550;
/// The first telemetry port of the other instances. It is clear of every
/// PX4 port (14550-14609 for instances 0-29), so an Aviate session runs
/// beside PX4 ones; 14550 itself is also where PX4's instance-0 host
/// listens.
const SHIFTED_TELEMETRY_BASE: u16 = 14_700;

/// The loopback port instance `instance`'s telemetry stream uses.
///
/// Both ends move together: the FC streams to `AVIATE_TELEMETRY_ENDPOINT`
/// and the host listens on `PILOTAGE_AVIATE_ADDR`. The shared-memory
/// block between the FC and the gz plugin does not move: the FC opens
/// instance 0's block whatever the session's instance, so one Aviate
/// session runs per machine (see the process claim).
fn telemetry_port(instance: u16) -> u16 {
    if instance == 0 {
        DEFAULT_TELEMETRY_PORT
    } else {
        SHIFTED_TELEMETRY_BASE.saturating_add(instance)
    }
}

fn telemetry_endpoint(ctx: &SessionContext) -> String {
    format!("127.0.0.1:{}", telemetry_port(ctx.instance))
}

/// The X500's own control-feel law.
///
/// Balanced rather than agile or precision: it is the law an operator who
/// has asked for nothing should get.
///
/// This binds the law the vehicle STARTS on. A runtime feel-mode request
/// does not reach these files — it shapes one from the mode alone — so
/// asking for another mode moves the vehicle off its fitted law.
const CONTROL_FEEL_PROFILE: &str = "adapters/aviate/profiles/x500-shaped-balanced-v1.json";

/// The Aviate + Gazebo SITL backend.
#[derive(Debug)]
pub struct AviateGz;

impl SimBackend for AviateGz {
    fn name(&self) -> &'static str {
        BACKEND_NAME
    }

    fn host_adapter(&self) -> &'static str {
        "aviate"
    }

    fn control_feel_profile(&self) -> Option<&'static str> {
        Some(CONTROL_FEEL_PROFILE)
    }

    fn host_env(&self, ctx: &SessionContext) -> Vec<(String, String)> {
        let mut env = vec![
            // The camera sidecar discovers gz topics through this.
            ("GZ_IP".to_owned(), "127.0.0.1".to_owned()),
            // A sensor's topic is scoped by the world it is in. The
            // launcher is what chooses the world, so it is what says which
            // one rather than the host guessing a name.
            ("PILOTAGE_GZ_WORLD".to_owned(), WORLD_NAME.to_owned()),
            ("PILOTAGE_AVIATE_ADDR".to_owned(), telemetry_endpoint(ctx)),
        ];
        env.extend(super::gz::partition_env(ctx, BACKEND_NAME));
        env
    }

    fn claims(&self, ctx: &SessionContext) -> Vec<Claim> {
        let mut claims = vec![
            // The FC and the gz plugin meet in shared memory named for
            // simulator instance 0, whatever the session's instance: a
            // second FC would attach to the first one's block.
            Claim::Process("sitl-gazebo-x500"),
            Claim::Udp {
                port: telemetry_port(ctx.instance),
                holder: "session host Aviate telemetry link".to_owned(),
                remedy: Remedy::Instance,
            },
        ];
        claims.extend(super::gz::partition_claims(ctx, BACKEND_NAME));
        claims
    }

    fn plan(&self, ctx: &SessionContext) -> Result<Vec<Stage>, XtaskError> {
        plan_with_aviate_dir(ctx, &super::aviate_dir(&ctx.repo_root))
    }

    fn spawn(
        &self,
        ctx: &SessionContext,
        point: &super::SpawnPoint,
        stages: &mut [Stage],
    ) -> Result<(), XtaskError> {
        super::gz::spawn_in_world(ctx, point, stages)
    }

    fn stale_process_patterns(&self) -> Vec<&'static str> {
        vec!["gz sim", "sitl-gazebo-x500"]
    }

    fn reset(&self, repo_root: &Path) -> Result<(), XtaskError> {
        let script = repo_root.join("scripts/reset-flight-sim.sh");
        let status = std::process::Command::new("bash")
            .arg(&script)
            .arg(WORLD_NAME)
            .env("PATH", search_path())
            .status()
            .map_err(|source| XtaskError::Io {
                context: "running the reset script",
                source,
            })?;
        if status.success() {
            Ok(())
        } else {
            Err(XtaskError::CommandFailed {
                context: "reset script",
                status: status.to_string(),
            })
        }
    }
}

/// `PATH` for spawned tools, with Homebrew's prefix appended when it
/// exists (gz lives there on macOS; a login shell has it, a bare spawn
/// may not).
pub(crate) fn search_path() -> String {
    let inherited = std::env::var("PATH").unwrap_or_default();
    let brew = Path::new("/opt/homebrew/bin");
    if brew.is_dir() && !inherited.split(':').any(|p| p == "/opt/homebrew/bin") {
        format!("{inherited}:/opt/homebrew/bin")
    } else {
        inherited
    }
}

/// The testable core of [`AviateGz::plan`]: validates every artifact
/// with an actionable hint, then assembles the gz and FC stages.
fn plan_with_aviate_dir(ctx: &SessionContext, aviate: &Path) -> Result<Vec<Stage>, XtaskError> {
    if !aviate.is_dir() {
        return Err(XtaskError::MissingArtifact {
            what: "Aviate checkout",
            path: aviate.to_path_buf(),
            hint: "clone the sibling Aviate repository or set AVIATE_DIR",
        });
    }
    let plugin_dir = aviate.join("aviate-hal/xil/backends/gz/plugin/build");
    let plugin = plugin_dir.join("libAviateGzPlugin.dylib");
    let plugin_linux = plugin_dir.join("libAviateGzPlugin.so");
    if !plugin.is_file() && !plugin_linux.is_file() {
        return Err(XtaskError::MissingArtifact {
            what: "Aviate gz plugin",
            path: plugin,
            hint: "build it: cmake .. && make -j8 in aviate-hal/xil/backends/gz/plugin/build",
        });
    }
    let fc = aviate.join("target/debug/sitl-gazebo-x500");
    if !fc.is_file() {
        return Err(XtaskError::MissingArtifact {
            what: "Aviate SITL FC binary",
            path: fc,
            hint: "build it: cargo build --bin sitl-gazebo-x500 in the Aviate checkout",
        });
    }
    let world = ctx.repo_root.join(WORLD);
    if !world.is_file() {
        return Err(XtaskError::MissingArtifact {
            what: "flight-deck world",
            path: world,
            hint: "run from the Pilotage repository root",
        });
    }
    Ok(vec![
        gz_stage(ctx, aviate, &search_path(), &world),
        fc_stage(ctx, aviate, &search_path(), &fc),
    ])
}

fn gz_stage(ctx: &SessionContext, aviate: &Path, path: &str, world: &Path) -> Stage {
    let models = aviate.join("external/PX4-gazebo-models/models");
    let mut gz_env = vec![
        ("PATH".to_owned(), path.to_owned()),
        ("GZ_IP".to_owned(), "127.0.0.1".to_owned()),
        (
            "GZ_SIM_SYSTEM_PLUGIN_PATH".to_owned(),
            aviate
                .join("aviate-hal/xil/backends/gz/plugin/build")
                .display()
                .to_string(),
        ),
        (
            "GZ_SIM_RESOURCE_PATH".to_owned(),
            format!(
                "{}:{}:{}",
                models.display(),
                ctx.repo_root.join("sim/worlds").display(),
                ctx.repo_root.join("sim/models").display()
            ),
        ),
    ];
    gz_env.extend(super::gz::partition_env(ctx, BACKEND_NAME));
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
            // Cleared so gz selects the EGL backend instead of an X
            // display that is not there.
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

fn fc_stage(ctx: &SessionContext, aviate: &Path, path: &str, fc: &Path) -> Stage {
    let mut env = vec![
        ("PATH".to_owned(), path.to_owned()),
        ("GZ_IP".to_owned(), "127.0.0.1".to_owned()),
        (
            "AVIATE_TELEMETRY_ENDPOINT".to_owned(),
            telemetry_endpoint(ctx),
        ),
    ];
    env.extend(super::gz::partition_env(ctx, BACKEND_NAME));
    Stage {
        spec: ProcessSpec {
            name: "flight-controller",
            program: fc.display().to_string(),
            args: vec![],
            cwd: Some(aviate.to_path_buf()),
            env,
            remove_env: vec![],
            log_path: stage_log(&ctx.log_dir, "flight-controller"),
        },
        readiness: Readiness::LogContains {
            needle: "-> Ready",
            timeout_s: 30,
        },
    }
}

#[cfg(test)]
mod tests;
