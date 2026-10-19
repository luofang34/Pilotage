//! The Betaflight SITL backend: one `pilotage-multirotor-fdm` process that
//! runs `betaflight_SITL.elf` as its child and closes its motor-to-sensor
//! loop. The sidecar owns the Betaflight process because the two start as
//! a unit: it loads `sim/betaflight/sitl.cli`, relaunches the SITL when a
//! start fails, and prints its ready line only once motor commands flow.
//! The host's Betaflight adapter flies it over MSP on TCP 5761.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::{Placement, SessionContext, SimBackend, Stage};
use crate::cli::Profile;
use crate::conflicts::{Claim, Remedy};
use crate::error::XtaskError;
use crate::process::ProcessSpec;
use crate::readiness::{Readiness, stage_log};

/// The FC stage's name, which [`BetaflightSitl::spawn`] finds it by.
const FC_STAGE: &str = "flight-controller";
/// The sidecar's line once Betaflight sends motor commands.
const READY_LINE: &str = "Betaflight is flying the model";
/// Betaflight SITL's fixed UDP ports: raw PWM out (no listener here, but a
/// second simulator listening there would read it), motors out (the
/// sidecar listens), state in, RC in. `sitl.c` has no option to move them.
/// Its TCP ports are 5760 + n for the UARTs with a serial function; the
/// configuration gives MSP to UART1 (5761, the adapter's link) and UART8
/// (5768, the sidecar's hang watch).
const SITL_UDP: [u16; 4] = [9_001, 9_002, 9_003, 9_004];
const SITL_MSP_TCP: u16 = 5_761;
const SITL_WATCH_TCP: u16 = 5_768;
const BUILD_HINT: &str = "build it in the Betaflight checkout (serial make; -j races the \
    submodule hydration): make TARGET=SITL EXTRA_FLAGS=\"-DENABLE_GAZEBO_BRIDGE=0\"";

/// The Betaflight SITL backend.
#[derive(Debug)]
pub struct BetaflightSitl;

/// Where the Betaflight checkout lives: `BETAFLIGHT_DIR`, else
/// `../betaflight` next to this repository.
fn betaflight_dir(repo_root: &Path) -> PathBuf {
    std::env::var_os("BETAFLIGHT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root.join("../betaflight"))
}

fn sidecar_binary(repo_root: &Path) -> PathBuf {
    repo_root.join("target/release/pilotage-multirotor-fdm")
}

impl SimBackend for BetaflightSitl {
    fn name(&self) -> &'static str {
        "betaflight-sitl"
    }

    fn host_adapter(&self) -> &'static str {
        "betaflight"
    }

    fn host_env(&self, _ctx: &SessionContext) -> Vec<(String, String)> {
        vec![
            (
                "PILOTAGE_BETAFLIGHT_PROFILE".to_owned(),
                "simulation".to_owned(),
            ),
            (
                "PILOTAGE_BETAFLIGHT_MSP_ADDR".to_owned(),
                format!("127.0.0.1:{SITL_MSP_TCP}"),
            ),
        ]
    }

    fn prepare(&self, ctx: &SessionContext) -> Result<(), XtaskError> {
        let status = Command::new("cargo")
            .args(["build", "--release", "-p", "pilotage-multirotor-fdm"])
            .current_dir(&ctx.repo_root)
            .status()
            .map_err(|source| XtaskError::Io {
                context: "building the multirotor flight-dynamics sidecar",
                source,
            })?;
        if status.success() {
            Ok(())
        } else {
            Err(XtaskError::CommandFailed {
                context: "cargo build --release -p pilotage-multirotor-fdm",
                status: status.to_string(),
            })
        }
    }

    fn plan(&self, ctx: &SessionContext) -> Result<Vec<Stage>, XtaskError> {
        plan_with_betaflight_dir(ctx, &betaflight_dir(&ctx.repo_root))
    }

    fn spawn(
        &self,
        _ctx: &SessionContext,
        point: &Placement,
        stages: &mut [Stage],
    ) -> Result<(), XtaskError> {
        let stage = stages
            .iter_mut()
            .find(|stage| stage.spec.name == FC_STAGE)
            .ok_or_else(|| XtaskError::Usage {
                message: "the Betaflight SITL plan has no flight-controller stage".to_owned(),
            })?;
        for (flag, value) in origin_args(point) {
            set_flag(&mut stage.spec.args, flag, value)?;
        }
        Ok(())
    }

    /// The fixed ports make Betaflight one per machine at any instance
    /// number: a fleet slot may take any instance, and a second Betaflight
    /// session fails on these claims.
    fn claims(&self, _ctx: &SessionContext) -> Vec<Claim> {
        let holder = "Betaflight SITL (one per machine: its ports are fixed)".to_owned();
        let mut claims: Vec<Claim> = SITL_UDP
            .map(|port| Claim::Udp {
                port,
                holder: holder.clone(),
                remedy: Remedy::StopOther,
            })
            .into();
        for port in [SITL_MSP_TCP, SITL_WATCH_TCP] {
            claims.push(Claim::Tcp {
                port,
                holder: holder.clone(),
                remedy: Remedy::StopOther,
            });
        }
        claims.extend(
            self.stale_process_patterns()
                .into_iter()
                .map(Claim::Process),
        );
        claims
    }

    fn stale_process_patterns(&self) -> Vec<&'static str> {
        vec!["betaflight_SITL.elf", "pilotage-multirotor-fdm"]
    }

    fn reset(&self, _repo_root: &Path) -> Result<(), XtaskError> {
        Err(XtaskError::Usage {
            message: "the betaflight-sitl backend has no reset; restart the session".to_owned(),
        })
    }
}

/// A plain `sim` session's spawn point: the second place on the grass
/// of Zurich, apart from a vehicle at the origin and an ArduPilot one at
/// the first place.
fn default_spawn() -> Placement {
    crate::site::ZURICH.placement(crate::site::Ground::Field, 1)
}

/// The sidecar options that put the model at `point`. Its local frame is
/// tangent there, so the spawn offset is zero; the origin altitude sets
/// the GNSS altitude and the barometer's pressure.
fn origin_args(point: &Placement) -> [(&'static str, String); 6] {
    [
        ("--origin-lat", format!("{:.7}", point.latitude_deg)),
        ("--origin-lon", format!("{:.7}", point.longitude_deg)),
        ("--origin-alt", format!("{:.2}", point.elevation_m)),
        ("--heading-deg", format!("{:.2}", point.heading_deg)),
        ("--north", "0".to_owned()),
        ("--east", "0".to_owned()),
    ]
}

/// Replaces the value after `flag` in `args`.
fn set_flag(args: &mut [String], flag: &str, value: String) -> Result<(), XtaskError> {
    let slot = args
        .iter()
        .position(|arg| arg == flag)
        .and_then(|at| args.get_mut(at + 1))
        .ok_or_else(|| XtaskError::Usage {
            message: format!("the Betaflight SITL stage has no {flag} to place"),
        })?;
    *slot = value;
    Ok(())
}

/// The testable core of [`BetaflightSitl::plan`].
fn plan_with_betaflight_dir(
    ctx: &SessionContext,
    betaflight: &Path,
) -> Result<Vec<Stage>, XtaskError> {
    if ctx.profile != Profile::Simulation {
        return Err(XtaskError::Usage {
            message: format!(
                "the betaflight-sitl backend supports only --profile simulation (got {:?})",
                ctx.profile
            ),
        });
    }
    let elf = betaflight.join("obj/main/betaflight_SITL.elf");
    if !elf.is_file() {
        return Err(XtaskError::MissingArtifact {
            what: "Betaflight SITL executable",
            path: elf,
            hint: BUILD_HINT,
        });
    }
    // Betaflight keeps eeprom.bin in its working directory; the session's
    // own log directory keeps it apart from any other checkout's.
    let work_dir = ctx.log_dir.join("betaflight");
    std::fs::create_dir_all(&work_dir).map_err(|source| XtaskError::Io {
        context: "creating the Betaflight SITL working directory",
        source,
    })?;
    let config = ctx.repo_root.join("sim/betaflight/sitl.cli");
    let mut args = [
        "--betaflight",
        &elf.display().to_string(),
        "--betaflight-dir",
        &work_dir.display().to_string(),
        "--betaflight-config",
        &config.display().to_string(),
    ]
    .map(str::to_owned)
    .to_vec();
    for (flag, value) in origin_args(&default_spawn()) {
        args.extend([flag.to_owned(), value]);
    }
    Ok(vec![Stage {
        spec: ProcessSpec {
            name: FC_STAGE,
            program: sidecar_binary(&ctx.repo_root).display().to_string(),
            args,
            cwd: Some(work_dir),
            env: vec![],
            remove_env: vec![],
            log_path: stage_log(&ctx.log_dir, FC_STAGE),
        },
        readiness: Readiness::LogContains {
            needle: READY_LINE,
            timeout_s: ctx.ready_timeout_s,
        },
    }])
}

#[cfg(test)]
mod tests;
