//! The Aviate + JSBSim SITL backend: the JSBSim sidecar flies the x500 on
//! the shared world, and Aviate's `sitl-jsbsim-x500` flight controller
//! dials it over the same MAVLink simulator link PX4 uses (TCP 4560 +
//! instance), taking each truth state and answering with its rotor
//! commands.
//!
//! Two stages, in launch order: the sidecar, ready once it listens, then
//! the flight controller, ready once its kernel is. Unlike `aviate-gz`
//! nothing is shared machine-wide: every port follows the instance, so
//! Aviate sessions run side by side.

use std::path::{Path, PathBuf};

use super::aviate_gz::{CONTROL_FEEL_PROFILE, telemetry_port};
use super::px4_gz::ports::MAX_INSTANCE;
use super::px4_jsbsim::{
    JsbsimModel, build_sidecar, place_sidecar, sidecar_stage, simulator_port, truth_port,
};
use super::{Placement, SessionContext, SimBackend, Stage};
use crate::cli::Profile;
use crate::conflicts::{Claim, Remedy};
use crate::error::XtaskError;
use crate::jsbsim::{JsbsimInstall, jsbsim_dir};
use crate::process::ProcessSpec;
use crate::readiness::{Readiness, stage_log};

/// This backend's `--fc` name.
const BACKEND_NAME: &str = "aviate-jsbsim";
/// The flight controller's binary in the Aviate checkout.
const FC_BINARY: &str = "sitl-jsbsim-x500";
/// The vehicle the sidecar flies for it.
const MODEL: JsbsimModel = JsbsimModel::Quad;
/// The line Aviate's runner logs once the kernel is ready to arm.
const FC_READY: &str = "-> Ready";
/// The first port of Aviate's XIL instance 0; each instance takes
/// [`XIL_STRIDE`] ports from there, and its MAVLink command link is the
/// first of them (Aviate's `XilNetConfig` defaults).
const XIL_BASE_PORT: u16 = 20_000;
const XIL_STRIDE: u16 = 16;

/// The UDP port instance `instance`'s flight controller takes commands on.
pub(crate) fn command_port(instance: u16) -> u16 {
    XIL_BASE_PORT + instance * XIL_STRIDE
}

/// The Aviate + JSBSim backend.
#[derive(Debug)]
pub struct AviateJsbsim;

impl SimBackend for AviateJsbsim {
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
        let instance = ctx.instance;
        vec![
            (
                "PILOTAGE_AVIATE_ADDR".to_owned(),
                format!("127.0.0.1:{}", telemetry_port(instance)),
            ),
            (
                "PILOTAGE_AVIATE_FC_ADDR".to_owned(),
                format!("127.0.0.1:{}", command_port(instance)),
            ),
            // Truth is the sidecar's, on the host's own lane: Gazebo's shm
            // block, when another session runs one, is another vehicle.
            ("PILOTAGE_AVIATE_TRUTH".to_owned(), "host-lane".to_owned()),
            (
                "PILOTAGE_SIM_TRUTH_ADDR".to_owned(),
                format!("127.0.0.1:{}", truth_port(instance)),
            ),
        ]
    }

    fn prepare(&self, ctx: &SessionContext) -> Result<(), XtaskError> {
        build_sidecar(ctx)
    }

    fn plan(&self, ctx: &SessionContext) -> Result<Vec<Stage>, XtaskError> {
        plan_with_dirs(ctx, &super::aviate_dir(&ctx.repo_root), &jsbsim_dir()?)
    }

    fn spawn(
        &self,
        _ctx: &SessionContext,
        point: &Placement,
        stages: &mut [Stage],
    ) -> Result<(), XtaskError> {
        place_sidecar(point, stages)?;
        state_origin(point, stages)
    }

    fn ground(&self) -> crate::site::Ground {
        MODEL.ground()
    }

    fn claims(&self, ctx: &SessionContext) -> Vec<Claim> {
        let instance = ctx.instance;
        vec![
            Claim::Tcp {
                port: simulator_port(instance),
                holder: format!("JSBSim sidecar for Aviate instance {instance}"),
                remedy: Remedy::Instance,
            },
            Claim::Udp {
                port: command_port(instance),
                holder: format!("Aviate instance {instance} command link"),
                remedy: Remedy::Instance,
            },
            Claim::Udp {
                port: telemetry_port(instance),
                holder: "session host Aviate telemetry link".to_owned(),
                remedy: Remedy::Instance,
            },
            Claim::Udp {
                port: truth_port(instance),
                holder: format!("session host truth lane for Aviate instance {instance}"),
                remedy: Remedy::Instance,
            },
        ]
    }

    fn stale_process_patterns(&self) -> Vec<&'static str> {
        vec![FC_BINARY, "pilotage-jsbsim-sidecar"]
    }

    fn reset(&self, repo_root: &Path) -> Result<(), XtaskError> {
        self.reset_instance(repo_root, 0)
    }

    /// A fresh flight controller redials, and the sidecar answers a new
    /// connection with a fresh vehicle at the start point.
    fn reset_instance(&self, repo_root: &Path, instance: u16) -> Result<(), XtaskError> {
        super::px4_gz::reset::restart_port_holder(
            repo_root,
            instance,
            command_port(instance),
            FC_BINARY,
        )
    }
}

/// Tells the flight controller where its world is: it makes its GNSS fix
/// and baro reading from the sidecar's truth around an origin it cannot
/// read from the link, so the vehicle's placement states it, at the
/// site's elevation.
fn state_origin(point: &Placement, stages: &mut [Stage]) -> Result<(), XtaskError> {
    let fc = stages
        .iter_mut()
        .find(|stage| stage.spec.name == "flight-controller")
        .ok_or_else(|| XtaskError::Usage {
            message: "the aviate-jsbsim plan has no flight-controller stage".to_owned(),
        })?;
    let origin = origin_value(point);
    match fc
        .spec
        .env
        .iter_mut()
        .find(|(key, _)| key == "AVIATE_GNSS_ORIGIN")
    {
        Some((_, value)) => *value = origin,
        None => fc.spec.env.push(("AVIATE_GNSS_ORIGIN".to_owned(), origin)),
    }
    Ok(())
}

/// `AVIATE_GNSS_ORIGIN`'s `latitude_deg,longitude_deg,elevation_m`.
fn origin_value(point: &Placement) -> String {
    format!(
        "{:.7},{:.7},{:.2}",
        point.latitude_deg, point.longitude_deg, point.elevation_m
    )
}

/// The testable core of [`AviateJsbsim::plan`]: validates every artifact
/// with an actionable hint, then plans the sidecar and the flight
/// controller.
fn plan_with_dirs(
    ctx: &SessionContext,
    aviate: &Path,
    jsbsim: &Path,
) -> Result<Vec<Stage>, XtaskError> {
    if ctx.profile != Profile::Simulation {
        return Err(XtaskError::Usage {
            message: format!(
                "the aviate-jsbsim backend supports only --profile simulation (got {:?})",
                ctx.profile
            ),
        });
    }
    if ctx.instance > MAX_INSTANCE {
        return Err(XtaskError::Usage {
            message: format!(
                "the aviate-jsbsim backend runs instances 0-{MAX_INSTANCE} (got {})",
                ctx.instance
            ),
        });
    }
    if !aviate.is_dir() {
        return Err(XtaskError::MissingArtifact {
            what: "Aviate checkout",
            path: aviate.to_path_buf(),
            hint: "clone the sibling Aviate repository or set AVIATE_DIR",
        });
    }
    let fc = fc_binary(aviate);
    if !fc.is_file() {
        return Err(XtaskError::MissingArtifact {
            what: "Aviate JSBSim SITL FC binary",
            path: fc,
            hint: "build it: cargo build --bin sitl-jsbsim-x500 in the Aviate checkout",
        });
    }
    let install = JsbsimInstall::validate(jsbsim)?;
    let mut stages = vec![
        sidecar_stage(ctx, &install, MODEL, None)?,
        fc_stage(ctx, aviate, &fc),
    ];
    // A plain `sim` session stands on the default site; a fleet moves it.
    let start = crate::site::place(&crate::site::DEFAULT_SITE, MODEL.ground(), 0)?;
    state_origin(&start, &mut stages)?;
    Ok(stages)
}

fn fc_binary(aviate: &Path) -> PathBuf {
    aviate.join("target/debug").join(FC_BINARY)
}

fn fc_stage(ctx: &SessionContext, aviate: &Path, fc: &Path) -> Stage {
    let env = vec![
        ("AVIATE_SITL_INSTANCE".to_owned(), ctx.instance.to_string()),
        (
            "AVIATE_TELEMETRY_ENDPOINT".to_owned(),
            format!("127.0.0.1:{}", telemetry_port(ctx.instance)),
        ),
    ];
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
            needle: FC_READY,
            timeout_s: ctx.ready_timeout_s,
        },
    }
}

#[cfg(test)]
mod tests;
