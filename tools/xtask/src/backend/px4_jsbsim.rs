//! The PX4 + JSBSim SITL backend: the JSBSim sidecar flies the vehicle
//! on the shared world, and PX4 SITL connects to it through PX4's own
//! MAVLink simulator link (`simulator_mavlink`, TCP 4560 + instance).
//!
//! Two stages, in launch order: the sidecar, ready once it listens, then
//! px4 with the airframe PX4 ships for a JSBSim vehicle. The PX4 adapter
//! sees the same MAVLink links as with any other simulator.

mod vehicle;

use std::path::{Path, PathBuf};

use super::px4_gz::ports::{self, MAX_INSTANCE};
use super::px4_gz::{px4_args, px4_dir, px4_rootfs};
use super::{Placement, SessionContext, SimBackend, Stage};
use crate::cli::Profile;
use crate::conflicts::{Claim, Remedy};
use crate::error::XtaskError;
use crate::jsbsim::{JsbsimInstall, jsbsim_dir};
use crate::process::ProcessSpec;
use crate::readiness::{Readiness, stage_log};

pub(crate) use vehicle::JsbsimModel;

/// This backend's `--fc` name.
const BACKEND_NAME: &str = "px4-jsbsim";
/// The sidecar's stage name, which [`Px4Jsbsim::spawn`] finds it by.
const SIDECAR_STAGE: &str = "jsbsim";
/// The line the sidecar logs once it listens for PX4.
const SIDECAR_READY: &str = "jsbsim sidecar listening for PX4";
/// The parameters every px4-jsbsim session sets. PX4's logger writes a
/// full ulog from arming, and in lockstep a slow disk stalls the whole
/// flight controller on it: simulation time then crawls at a tenth of
/// real time. The sidecar's truth track and PX4's console log stand in.
const SESSION_PARAMS: [(&str, &str); 1] = [("SDLOG_MODE", "-1")];
/// PX4's simulator link port of instance 0.
const SIMULATOR_PORT_BASE: u16 = 4_560;

/// The PX4 + JSBSim backend, for one vehicle model.
#[derive(Debug, Default)]
pub struct Px4Jsbsim {
    model: JsbsimModel,
}

/// The TCP port PX4 instance `instance` connects its simulator link to.
pub(crate) fn simulator_port(instance: u16) -> u16 {
    SIMULATOR_PORT_BASE + instance
}

fn sidecar_binary(repo_root: &Path) -> PathBuf {
    repo_root.join("target/release/pilotage-jsbsim-sidecar")
}

impl SimBackend for Px4Jsbsim {
    fn name(&self) -> &'static str {
        BACKEND_NAME
    }

    fn host_adapter(&self) -> &'static str {
        "px4"
    }

    fn host_env(&self, ctx: &SessionContext) -> Vec<(String, String)> {
        let mut env = ports::host_env(ctx.instance, self.model.airframe());
        // Only a vehicle that carries a gimbal advertises the
        // vehicle.gimbal scope; the flag's absence means none.
        if self.model.gimbal().is_some() {
            env.push(("PILOTAGE_PX4_GIMBAL".to_owned(), "1".to_owned()));
        }
        env
    }

    fn prepare(&self, ctx: &SessionContext) -> Result<(), XtaskError> {
        let status = std::process::Command::new("cargo")
            .args(["build", "--release", "-p", "pilotage-jsbsim-sidecar"])
            .current_dir(&ctx.repo_root)
            .status()
            .map_err(|source| XtaskError::Io {
                context: "building the JSBSim sidecar",
                source,
            })?;
        if status.success() {
            Ok(())
        } else {
            Err(XtaskError::CommandFailed {
                context: "cargo build --release -p pilotage-jsbsim-sidecar",
                status: status.to_string(),
            })
        }
    }

    fn plan(&self, ctx: &SessionContext) -> Result<Vec<Stage>, XtaskError> {
        let jsbsim = jsbsim_dir()?;
        plan_with_dirs(ctx, &px4_dir(&ctx.repo_root), &jsbsim, self.model)
    }

    fn spawn(
        &self,
        _ctx: &SessionContext,
        point: &Placement,
        stages: &mut [Stage],
    ) -> Result<(), XtaskError> {
        let stage = stages
            .iter_mut()
            .find(|stage| stage.spec.name == SIDECAR_STAGE)
            .ok_or_else(|| XtaskError::Usage {
                message: "the px4-jsbsim plan has no jsbsim stage".to_owned(),
            })?;
        for (flag, value) in placement_args(point) {
            set_flag(&mut stage.spec.args, flag, value)?;
        }
        Ok(())
    }

    fn ground(&self) -> crate::site::Ground {
        self.model.ground()
    }

    fn select_model(&mut self, model: &str) -> Result<(), XtaskError> {
        self.model = JsbsimModel::parse(model)?;
        Ok(())
    }

    fn claims(&self, ctx: &SessionContext) -> Vec<Claim> {
        let instance = ctx.instance;
        let mut claims = ports::link_claims(instance);
        claims.push(Claim::Tcp {
            port: simulator_port(instance),
            holder: format!("JSBSim sidecar for PX4 instance {instance}"),
            remedy: Remedy::Instance,
        });
        claims
    }

    fn stale_process_patterns(&self) -> Vec<&'static str> {
        vec!["bin/px4", "pilotage-jsbsim-sidecar"]
    }

    fn reset(&self, repo_root: &Path) -> Result<(), XtaskError> {
        self.reset_instance(repo_root, 0)
    }

    /// A fresh px4 reconnects, and the sidecar answers a new connection
    /// with a fresh vehicle at the start point: the world resets with it.
    fn reset_instance(&self, repo_root: &Path, instance: u16) -> Result<(), XtaskError> {
        super::px4_gz::reset::restart_instance(repo_root, &px4_dir(repo_root), instance)
    }
}

/// The sidecar options that put the vehicle at `point`. The site's
/// declared elevation is the ground where the world has no terrain.
fn placement_args(point: &Placement) -> [(&'static str, String); 4] {
    [
        ("--latitude-deg", format!("{:.7}", point.latitude_deg)),
        ("--longitude-deg", format!("{:.7}", point.longitude_deg)),
        ("--heading-deg", format!("{:.2}", point.heading_deg)),
        ("--site-elevation-m", format!("{:.2}", point.elevation_m)),
    ]
}

/// Replaces the value after `flag` in `args`.
fn set_flag(args: &mut [String], flag: &str, value: String) -> Result<(), XtaskError> {
    let slot = args
        .iter()
        .position(|arg| arg == flag)
        .and_then(|at| args.get_mut(at + 1))
        .ok_or_else(|| XtaskError::Usage {
            message: format!("the jsbsim stage has no {flag} to place"),
        })?;
    *slot = value;
    Ok(())
}

/// The testable core of [`Px4Jsbsim::plan`]: validates every artifact
/// with an actionable hint, then plans the sidecar and px4 stages.
fn plan_with_dirs(
    ctx: &SessionContext,
    px4: &Path,
    jsbsim: &Path,
    model: JsbsimModel,
) -> Result<Vec<Stage>, XtaskError> {
    if ctx.profile != Profile::Simulation {
        return Err(XtaskError::Usage {
            message: format!(
                "the px4-jsbsim backend supports only --profile simulation (got {:?})",
                ctx.profile
            ),
        });
    }
    if ctx.instance > MAX_INSTANCE {
        return Err(XtaskError::Usage {
            message: format!(
                "the px4-jsbsim backend runs instances 0-{MAX_INSTANCE} (got {})",
                ctx.instance
            ),
        });
    }
    let binary = px4.join("build/px4_sitl_default/bin/px4");
    if !binary.is_file() {
        return Err(XtaskError::MissingArtifact {
            what: "PX4 SITL binary",
            path: binary,
            hint: "build it: make px4_sitl in the PX4-Autopilot checkout (or set PX4_DIR)",
        });
    }
    let install = JsbsimInstall::validate(jsbsim)?;
    Ok(vec![
        sidecar_stage(ctx, &install, model),
        px4_stage(ctx, px4, &binary, model)?,
    ])
}

fn sidecar_stage(ctx: &SessionContext, install: &JsbsimInstall, model: JsbsimModel) -> Stage {
    let mut args = vec![
        "--port".to_owned(),
        simulator_port(ctx.instance).to_string(),
        "--aircraft".to_owned(),
        model.sidecar_aircraft().to_owned(),
        "--jsbsim-root".to_owned(),
        install.root.display().to_string(),
    ];
    if let Some(travel) = model.gimbal() {
        args.extend([
            "--gimbal".to_owned(),
            [
                travel.range_roll,
                travel.max_pitch,
                travel.min_pitch,
                travel.range_yaw,
            ]
            .join(","),
        ]);
    }
    // A plain `sim` session stands on the default site; a fleet moves it.
    let start = crate::site::DEFAULT_SITE.placement(model.ground(), 0);
    for (flag, value) in placement_args(&start) {
        args.extend([flag.to_owned(), value]);
    }
    Stage {
        spec: ProcessSpec {
            name: SIDECAR_STAGE,
            program: sidecar_binary(&ctx.repo_root).display().to_string(),
            args,
            cwd: None,
            env: vec![],
            remove_env: vec![],
            log_path: stage_log(&ctx.log_dir, SIDECAR_STAGE),
        },
        readiness: Readiness::LogContains {
            needle: SIDECAR_READY,
            timeout_s: ctx.ready_timeout_s,
        },
    }
}

fn px4_stage(
    ctx: &SessionContext,
    px4: &Path,
    binary: &Path,
    model: JsbsimModel,
) -> Result<Stage, XtaskError> {
    let rootfs = px4_rootfs(px4, ctx.instance);
    std::fs::create_dir_all(&rootfs).map_err(|source| XtaskError::Io {
        context: "creating the PX4 rootfs directory",
        source,
    })?;
    let mut env = vec![(
        "PX4_SYS_AUTOSTART".to_owned(),
        model.sys_autostart().to_owned(),
    )];
    env.extend(
        SESSION_PARAMS
            .iter()
            .chain(model.params())
            .map(|(name, value)| (format!("PX4_PARAM_{name}"), (*value).to_owned())),
    );
    Ok(Stage {
        spec: ProcessSpec {
            name: "flight-controller",
            program: binary.display().to_string(),
            args: px4_args(px4, ctx.instance),
            cwd: Some(rootfs),
            env,
            // A gz session's variables would turn the airframe's
            // simulator choice to gz.
            remove_env: vec!["PX4_SIMULATOR", "PX4_SIM_MODEL", "PX4_GZ_STANDALONE"],
            log_path: stage_log(&ctx.log_dir, "flight-controller"),
        },
        // As for px4-gz: boot completion is the honest FC-stage signal.
        readiness: Readiness::LogContains {
            needle: "Startup script returned successfully",
            timeout_s: ctx.ready_timeout_s,
        },
    })
}

#[cfg(test)]
mod tests;
