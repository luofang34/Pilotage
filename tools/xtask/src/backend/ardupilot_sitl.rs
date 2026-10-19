//! The ArduCopter SITL backend: one `arducopter` process with its built-in
//! quad physics, no external simulator and no Python. Its serial 0 sends
//! MAVLink over UDP to the host's ArduPilot adapter, which answers the
//! address the FC sends from.

use std::path::{Path, PathBuf};

use super::{SessionContext, SimBackend, SpawnPoint, Stage};
use crate::cli::Profile;
use crate::conflicts::{Claim, Remedy};
use crate::error::XtaskError;
use crate::process::ProcessSpec;
use crate::readiness::{Readiness, stage_log};

/// The FC stage's name, which [`ArduPilotSitl::spawn`] finds it by.
const FC_STAGE: &str = "flight-controller";
/// The UDP port the adapter of instance 0 listens on; instance n adds n.
/// Clear of the PX4 ranges (14550+n, 14580+n, 18570+n).
const MAVLINK_PORT_BASE: u16 = 14_700;
/// SITL's own ports for instance 0; `-I<n>` adds 10·n to each.
const SITL_RC_IN_UDP: u16 = 5_501;
const SITL_SERIAL_TCP: [u16; 2] = [5_762, 5_763];
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

fn mavlink_port(instance: u16) -> u16 {
    MAVLINK_PORT_BASE + instance
}

/// A plain `sim` session's home: the first place on the grass of
/// Zurich, where PX4's own worlds stand, apart from a vehicle at the
/// origin.
fn default_home() -> SpawnPoint {
    crate::site::ZURICH.spawn_point(crate::site::Ground::Field, 0)
}

/// SITL's `--home latitude,longitude,altitude_msl,heading`.
fn home_arg(point: &SpawnPoint) -> String {
    format!(
        "{:.7},{:.7},{},{}",
        point.latitude_deg, point.longitude_deg, point.elevation_m, point.heading_deg
    )
}

impl SimBackend for ArduPilotSitl {
    fn name(&self) -> &'static str {
        "ardupilot-sitl"
    }

    fn host_adapter(&self) -> &'static str {
        "ardupilot"
    }

    fn host_env(&self, ctx: &SessionContext) -> Vec<(String, String)> {
        vec![
            (
                "PILOTAGE_ARDUPILOT_PROFILE".to_owned(),
                "simulation".to_owned(),
            ),
            (
                "PILOTAGE_ARDUPILOT_ADDR".to_owned(),
                format!("127.0.0.1:{}", mavlink_port(ctx.instance)),
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

    fn spawn(
        &self,
        _ctx: &SessionContext,
        point: &SpawnPoint,
        stages: &mut [Stage],
    ) -> Result<(), XtaskError> {
        let home = stages
            .iter_mut()
            .filter(|stage| stage.spec.name == FC_STAGE)
            .flat_map(|stage| stage.spec.args.iter_mut())
            .skip_while(|arg| arg.as_str() != "--home")
            .nth(1)
            .ok_or_else(|| XtaskError::Usage {
                message: "the ArduPilot SITL stage has no --home to place".to_owned(),
            })?;
        *home = home_arg(point);
        Ok(())
    }

    fn claims(&self, ctx: &SessionContext) -> Vec<Claim> {
        let instance = ctx.instance;
        let shift = 10 * instance;
        let holder = format!("ArduPilot SITL instance {instance}");
        let mut claims = vec![
            Claim::Udp {
                port: mavlink_port(instance),
                holder: format!("session host MAVLink link for ArduPilot instance {instance}"),
                remedy: Remedy::Instance,
            },
            Claim::Udp {
                port: SITL_RC_IN_UDP + shift,
                holder: holder.clone(),
                remedy: Remedy::Instance,
            },
        ];
        claims.extend(SITL_SERIAL_TCP.map(|port| Claim::Tcp {
            port: port + shift,
            holder: holder.clone(),
            remedy: Remedy::Instance,
        }));
        claims
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
    // SITL writes eeprom.bin and logs into its working directory; the
    // session's own log directory keeps sessions apart.
    let cwd = ctx.log_dir.join(format!("ardupilot-{}", ctx.instance));
    std::fs::create_dir_all(&cwd).map_err(|source| XtaskError::Io {
        context: "creating the ArduPilot SITL working directory",
        source,
    })?;
    Ok(vec![Stage {
        spec: ProcessSpec {
            name: FC_STAGE,
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
                format!("-I{}", ctx.instance),
                "--home".to_owned(),
                home_arg(&default_home()),
                "--serial0".to_owned(),
                format!("udpclient:127.0.0.1:{}", mavlink_port(ctx.instance)),
            ],
            cwd: Some(cwd),
            env: vec![],
            remove_env: vec![],
            log_path: stage_log(&ctx.log_dir, FC_STAGE),
        },
        readiness: Readiness::LogContains {
            needle: "UDP connection",
            timeout_s: ctx.ready_timeout_s,
        },
    }])
}

#[cfg(test)]
mod tests;
