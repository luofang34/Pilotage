//! The ArduCopter SITL backend: one `arducopter` process with its built-in
//! quad physics, no external simulator and no Python. Its serial 0 sends
//! MAVLink over UDP to the host's ArduPilot adapter, which answers the
//! address the FC sends from.
//!
//! The vehicle spawns east of the shared demo origin so it stands apart
//! from the other demo vehicles on the map.

use std::path::{Path, PathBuf};

use super::{SessionContext, SimBackend, Stage};
use crate::cli::Profile;
use crate::error::XtaskError;
use crate::process::ProcessSpec;
use crate::readiness::{Readiness, stage_log};

/// The SITL instance: `-I<n>` adds 10·n to every SITL port.
const INSTANCE: u16 = 0;
/// The UDP port the adapter listens on for this instance. Clear of
/// 14550, which the PX4 adapter binds.
const MAVLINK_PORT: u16 = 14_590 + INSTANCE;
/// Home: the demo origin (47.397742, 8.545594) moved 30 m east, at the
/// origin's 488 m altitude, heading north.
const HOME: &str = "47.397742,8.545992,488,0";
/// How to build the binary. The linker flag is for Apple's current
/// linker, which refuses the unaligned pointers ArduPilot's packed
/// structs produce under chained fixups.
const BUILD_HINT: &str = "build it in the ArduPilot checkout: \
    uv venv .venv --python 3.12 && VIRTUAL_ENV=$PWD/.venv uv pip install \
    empy==3.3.4 pexpect future intelhex dronecan pymavlink lxml && \
    PATH=$PWD/.venv/bin:$PATH LDFLAGS=-Wl,-no_fixup_chains ./waf configure --board sitl && \
    PATH=$PWD/.venv/bin:$PATH ./waf copter";

/// The ArduCopter SITL backend.
#[derive(Debug)]
pub struct ArduPilotSitl;

/// Where the ArduPilot checkout lives: `ARDUPILOT_DIR`, else
/// `../ardupilot` next to this repository.
pub(crate) fn ardupilot_dir(repo_root: &Path) -> PathBuf {
    std::env::var_os("ARDUPILOT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root.join("../ardupilot"))
}

impl SimBackend for ArduPilotSitl {
    fn name(&self) -> &'static str {
        "ardupilot-sitl"
    }

    fn host_adapter(&self) -> &'static str {
        "ardupilot"
    }

    fn host_env(&self, _ctx: &SessionContext) -> Vec<(String, String)> {
        vec![
            (
                "PILOTAGE_ARDUPILOT_PROFILE".to_owned(),
                "simulation".to_owned(),
            ),
            (
                "PILOTAGE_ARDUPILOT_ADDR".to_owned(),
                format!("127.0.0.1:{MAVLINK_PORT}"),
            ),
        ]
    }

    fn plan(&self, ctx: &SessionContext) -> Result<Vec<Stage>, XtaskError> {
        if ctx.profile != Profile::Simulation {
            return Err(XtaskError::Usage {
                message: format!(
                    "the ardupilot-sitl backend supports only --profile simulation (got {:?})",
                    ctx.profile
                ),
            });
        }
        plan_with_ardupilot_dir(ctx, &ardupilot_dir(&ctx.repo_root))
    }

    fn stale_process_patterns(&self) -> Vec<&'static str> {
        vec!["bin/arducopter"]
    }

    fn reset(&self, _repo_root: &Path) -> Result<(), XtaskError> {
        Err(XtaskError::Usage {
            message: "the ardupilot-sitl backend has no reset; restart the session".to_owned(),
        })
    }
}

/// The testable core of [`ArduPilotSitl::plan`].
fn plan_with_ardupilot_dir(
    ctx: &SessionContext,
    ardupilot: &Path,
) -> Result<Vec<Stage>, XtaskError> {
    let binary = ardupilot.join("build/sitl/bin/arducopter");
    if !binary.is_file() {
        return Err(XtaskError::MissingArtifact {
            what: "ArduCopter SITL binary",
            path: binary,
            hint: BUILD_HINT,
        });
    }
    let defaults = ardupilot.join("Tools/autotest/default_params/copter.parm");
    if !defaults.is_file() {
        return Err(XtaskError::MissingArtifact {
            what: "ArduCopter SITL default parameters",
            path: defaults,
            hint: "the ArduPilot checkout is incomplete; clone it again",
        });
    }
    // SITL writes eeprom.bin and logs into its working directory: one per
    // instance, under this session's logs, so sessions never share
    // parameters.
    let cwd = ctx.log_dir.join(format!("ardupilot-{INSTANCE}"));
    std::fs::create_dir_all(&cwd).map_err(|source| XtaskError::Io {
        context: "creating the ArduPilot SITL working directory",
        source,
    })?;
    Ok(vec![Stage {
        spec: ProcessSpec {
            name: "flight-controller",
            program: binary.display().to_string(),
            args: vec![
                "--model".to_owned(),
                "+".to_owned(),
                "--speedup".to_owned(),
                "1".to_owned(),
                // Fresh parameters every session: the defaults below,
                // nothing a previous session left behind.
                "--wipe".to_owned(),
                "--defaults".to_owned(),
                defaults.display().to_string(),
                format!("-I{INSTANCE}"),
                "--home".to_owned(),
                HOME.to_owned(),
                "--serial0".to_owned(),
                format!("udpclient:127.0.0.1:{MAVLINK_PORT}"),
            ],
            cwd: Some(cwd),
            env: vec![],
            remove_env: vec![],
            log_path: stage_log(&ctx.log_dir, "flight-controller"),
        },
        readiness: Readiness::LogContains {
            needle: "UDP connection",
            timeout_s: 60,
        },
    }])
}

#[cfg(test)]
mod tests;
