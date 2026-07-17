//! The Aviate + Gazebo SITL backend: the proven flight-deck runbook as
//! code — headless gz with the Aviate plugin, then the SITL FC over the
//! versioned shm block, each gated on its own readiness signal.

use std::path::{Path, PathBuf};

use super::{SessionContext, SimBackend, Stage};
use crate::error::XtaskError;
use crate::process::ProcessSpec;
use crate::readiness::{Readiness, stage_log};

/// The world this backend launches; its `<world name>` is `aviate_sitl`
/// (also what the reset script targets).
const WORLD: &str = "sim/worlds/x500_flightdeck.sdf";
const WORLD_NAME: &str = "aviate_sitl";

/// The Aviate + Gazebo SITL backend.
#[derive(Debug)]
pub struct AviateGz;

impl SimBackend for AviateGz {
    fn name(&self) -> &'static str {
        "aviate"
    }

    fn host_adapter(&self) -> &'static str {
        "aviate"
    }

    fn host_env(&self, _ctx: &SessionContext) -> Vec<(String, String)> {
        // The camera sidecar discovers gz topics through this.
        vec![("GZ_IP".to_owned(), "127.0.0.1".to_owned())]
    }

    fn plan(&self, ctx: &SessionContext) -> Result<Vec<Stage>, XtaskError> {
        plan_with_aviate_dir(ctx, &aviate_dir(&ctx.repo_root))
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

/// Where the sibling Aviate checkout lives: `AVIATE_DIR`, else
/// `../Aviate` next to this repository. A directory convention, never a
/// source dependency.
fn aviate_dir(repo_root: &Path) -> PathBuf {
    std::env::var_os("AVIATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root.join("../Aviate"))
}

/// `PATH` for spawned tools, with Homebrew's prefix appended when it
/// exists (gz lives there on macOS; a login shell has it, a bare spawn
/// may not).
fn search_path() -> String {
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
    let gz_env = vec![
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
                "{}:{}",
                models.display(),
                ctx.repo_root.join("sim/worlds").display()
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
    Stage {
        spec: ProcessSpec {
            name: "flight-controller",
            program: fc.display().to_string(),
            args: vec![],
            cwd: Some(aviate.to_path_buf()),
            env: vec![
                ("PATH".to_owned(), path.to_owned()),
                ("GZ_IP".to_owned(), "127.0.0.1".to_owned()),
            ],
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
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use std::path::PathBuf;

    use super::plan_with_aviate_dir;
    use crate::backend::SessionContext;
    use crate::cli::Profile;
    use crate::error::XtaskError;

    fn context(repo_root: PathBuf) -> SessionContext {
        let log_dir = repo_root.join("target/xtask-sim");
        SessionContext {
            repo_root,
            host_port: 4433,
            viewer_port: 8080,
            profile: Profile::Simulation,
            log_dir,
        }
    }

    fn scaffold(tag: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("plt_xtask_{tag}_{}", std::process::id()));
        let repo = base.join("repo");
        let aviate = base.join("Aviate");
        std::fs::create_dir_all(repo.join("sim/worlds")).expect("repo dirs");
        std::fs::create_dir_all(aviate.join("aviate-hal/xil/backends/gz/plugin/build"))
            .expect("plugin dir");
        std::fs::create_dir_all(aviate.join("target/debug")).expect("fc dir");
        (repo, aviate)
    }

    #[test]
    fn missing_artifacts_fail_with_actionable_hints() {
        let (repo, aviate) = scaffold("hints");
        let ctx = context(repo.clone());

        let refusal = plan_with_aviate_dir(&ctx, &aviate.join("nowhere"));
        assert!(matches!(
            refusal,
            Err(XtaskError::MissingArtifact {
                what: "Aviate checkout",
                ..
            })
        ));

        let refusal = plan_with_aviate_dir(&ctx, &aviate);
        assert!(matches!(
            refusal,
            Err(XtaskError::MissingArtifact {
                what: "Aviate gz plugin",
                ..
            })
        ));

        std::fs::write(
            aviate.join("aviate-hal/xil/backends/gz/plugin/build/libAviateGzPlugin.dylib"),
            b"",
        )
        .expect("plugin");
        let refusal = plan_with_aviate_dir(&ctx, &aviate);
        assert!(matches!(
            refusal,
            Err(XtaskError::MissingArtifact {
                what: "Aviate SITL FC binary",
                ..
            })
        ));

        std::fs::write(aviate.join("target/debug/sitl-gazebo-x500"), b"").expect("fc");
        let refusal = plan_with_aviate_dir(&ctx, &aviate);
        assert!(matches!(
            refusal,
            Err(XtaskError::MissingArtifact {
                what: "flight-deck world",
                ..
            })
        ));
        std::fs::remove_dir_all(repo.parent().expect("base")).ok();
    }

    #[test]
    fn a_complete_checkout_plans_gz_then_fc_with_the_runbook_environment() {
        let (repo, aviate) = scaffold("plan");
        std::fs::write(
            aviate.join("aviate-hal/xil/backends/gz/plugin/build/libAviateGzPlugin.dylib"),
            b"",
        )
        .expect("plugin");
        std::fs::write(aviate.join("target/debug/sitl-gazebo-x500"), b"").expect("fc");
        std::fs::write(repo.join("sim/worlds/x500_flightdeck.sdf"), b"<sdf/>").expect("world");

        let ctx = context(repo.clone());
        let stages = plan_with_aviate_dir(&ctx, &aviate).expect("plans");
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0].spec.name, "gazebo");
        assert_eq!(stages[1].spec.name, "flight-controller");

        let env = |stage: usize, key: &str| -> String {
            stages[stage]
                .spec
                .env
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| panic!("{key} missing"))
        };
        assert!(env(0, "GZ_SIM_SYSTEM_PLUGIN_PATH").contains("plugin/build"));
        assert!(env(0, "GZ_SIM_RESOURCE_PATH").contains("PX4-gazebo-models"));
        assert!(env(0, "GZ_SIM_RESOURCE_PATH").contains("sim/worlds"));
        assert_eq!(env(0, "GZ_IP"), "127.0.0.1");
        assert!(
            stages[0].spec.remove_env.contains(&"DISPLAY"),
            "gz must run headless without an X display"
        );
        assert_eq!(env(1, "GZ_IP"), "127.0.0.1");
        std::fs::remove_dir_all(repo.parent().expect("base")).ok();
    }
}
