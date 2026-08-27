//! The Aviate + X-Plane SITL backend.
//!
//! Same simulator discipline as the PX4 X-Plane backend — X-Plane is an
//! operator-owned desktop application the launcher never manages — with
//! the Aviate flight controller in place of PX4. Aviate dials the same
//! bridge plugin on TCP 4560, so the two flight-controller families are
//! ALTERNATIVES on one simulator, never concurrent: the bridge serves
//! one connection.
//!
//! The payload view comes from the in-simulator camera plugin, which
//! dials the host. That is what makes the gimbal scope real here: the
//! adapter aims a rendered view, not a servo.

use std::path::{Path, PathBuf};

use super::xplane_simulator::{
    airframe_for, ensure_xplane_plugins_blocking, prepare_xplane_runtime_blocking,
    set_active_config_name, set_ground_sensor_contract, validate_xplane_install,
    verify_loaded_aircraft, xplane_root, xplane_running_blocking,
};
use super::{SessionContext, SimBackend, Stage};
use crate::cli::Profile;
use crate::error::XtaskError;
use crate::output::print_line;
use crate::process::ProcessSpec;
use crate::readiness::{Readiness, stage_log};

/// The Aviate application this backend runs: the Alia-250 lift rotors,
/// whose four-rotor quad-X arrangement the flight controller mixes.
const APP_BINARY: &str = "target/debug/sitl-xplane-alia250";

/// The px4xplane airframe whose channel map matches that application.
/// `PILOTAGE_XPLANE_AIRFRAME` overrides it, so a tuning session can fly
/// the same four-rotor channel map on a different simulator aircraft.
const AIRFRAME: &str = "alia250";

fn airframe_key() -> String {
    std::env::var("PILOTAGE_XPLANE_AIRFRAME").unwrap_or_else(|_| AIRFRAME.to_owned())
}

/// The Alia's own control-feel law.
///
/// This is the same artifact the host falls back to when nothing names
/// one, but it reaches the Alia there by being the single compiled-in
/// default rather than by being this aircraft's law — which is how the
/// X500 came to fly on it too. Named here, each backend answers for its
/// own vehicle.
const CONTROL_FEEL_PROFILE: &str = "adapters/aviate/profiles/alia250-legacy-v1.json";

/// The Aviate + X-Plane SITL backend.
#[derive(Debug)]
pub struct AviateXPlane;

impl SimBackend for AviateXPlane {
    fn name(&self) -> &'static str {
        "aviate-xplane"
    }

    fn host_adapter(&self) -> &'static str {
        "aviate"
    }

    fn host_env(&self, ctx: &SessionContext) -> Vec<(String, String)> {
        vec![
            (
                "PILOTAGE_AVIATE_PROFILE".to_owned(),
                ctx.profile.as_env_value().to_owned(),
            ),
            // The payload view comes from the in-simulator camera
            // plugin, which dials this host.
            (
                "PILOTAGE_AVIATE_CAMERA".to_owned(),
                "xplane-plugin".to_owned(),
            ),
            (
                "PILOTAGE_AVIATE_CONTROL_FEEL_PROFILE".to_owned(),
                ctx.repo_root
                    .join(CONTROL_FEEL_PROFILE)
                    .display()
                    .to_string(),
            ),
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
                    "the aviate-xplane backend supports only --profile simulation (got {:?})",
                    ctx.profile
                ),
            });
        }
        let airframe = airframe_for(Some(&airframe_key()))?;
        let root = xplane_root()?;
        validate_xplane_install(&root, airframe)?;
        let aviate = aviate_dir(&ctx.repo_root);
        let binary = aviate.join(APP_BINARY);
        if !binary.is_file() {
            return Err(XtaskError::MissingArtifact {
                what: "Aviate X-Plane SITL binary",
                path: binary,
                hint: "build it: cargo build -p aviate-app-sitl-xplane-alia250 \
                       in the Aviate checkout",
            });
        }
        // Aviate binds every Alia run to a verified runtime and will not start
        // without the document that names it. Producing it blocks until the
        // trial plugin inside X-Plane states its own identity, which is the
        // point: the run is bound to the simulator that is actually running,
        // not to one a launcher claimed was.
        let handshake = super::xplane_handshake::produce_blocking(
            &root,
            &aviate.join("presets/alia250-xplane.toml"),
            &ctx.log_dir,
        )?;
        Ok(vec![Stage {
            spec: ProcessSpec {
                name: "flight-controller",
                program: binary.display().to_string(),
                args: vec![
                    "--runtime-handshake".to_owned(),
                    handshake.display().to_string(),
                ],
                cwd: Some(aviate),
                env: vec![("RUST_LOG".to_owned(), "info".to_owned())],
                remove_env: vec![],
                log_path: stage_log(&ctx.log_dir, "flight-controller"),
            },
            // The flight controller dials the bridge and retries until
            // it answers, so the honest readiness signal is the link
            // coming up — not a boot line printed before it.
            readiness: Readiness::LogContains {
                needle: "HIL link up",
                timeout_s: 420,
            },
        }])
    }

    fn before_stage_restart(
        &self,
        ctx: &SessionContext,
        stage_name: &str,
    ) -> Option<Box<dyn FnOnce() -> Result<(), XtaskError> + Send>> {
        if stage_name != "flight-controller" {
            return None;
        }
        // The flight controller CLAIMS its handshake by deleting it, so the
        // document the plan wrote is gone by the time a reset restarts it. A
        // replacement handed that path would find nothing and never come up.
        // Re-verifying rather than re-writing is the point: the restarted
        // controller is bound to the simulator that is running NOW, which may
        // not be the one the first start verified.
        //
        // The paths are taken now and owned by the returned work, so it can
        // wait for the simulator without holding the runtime thread.
        let preset = aviate_dir(&ctx.repo_root).join("presets/alia250-xplane.toml");
        let log_dir = ctx.log_dir.clone();
        Some(Box::new(move || {
            let root = xplane_root()?;
            super::xplane_handshake::produce_blocking(&root, &preset, &log_dir)?;
            Ok(())
        }))
    }

    fn prepare(&self, ctx: &SessionContext) -> Result<(), XtaskError> {
        ensure_xplane_plugins_blocking(&ctx.repo_root, xplane_running_blocking()?)?;
        // The airframe the flight controller mixes decides which channel
        // map the bridge must load, so this backend PINS it rather than
        // trusting whatever the last session left behind.
        let Ok(airframe) = airframe_for(Some(&airframe_key())) else {
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
        set_active_config_name(&root, airframe, simulator_running)?;
        // Aviate's estimator consumes REAL sensors from boot to
        // touchdown; the bridge's fabricated ground-stationary contract
        // is a PX4-specific crutch this lane refuses.
        set_ground_sensor_contract(&root, false, simulator_running)?;
        prepare_xplane_runtime_blocking(
            &ctx.repo_root,
            &root,
            airframe,
            &ctx.log_dir,
            simulator_running,
        )?;
        if simulator_running {
            print_line("X-Plane weather is ready and the SITL listener is armed");
        }
        Ok(())
    }

    fn stale_process_patterns(&self) -> Vec<&'static str> {
        // X-Plane is deliberately absent: the launcher never owns it.
        vec!["sitl-xplane-alia250"]
    }

    fn reset(&self, repo_root: &Path) -> Result<(), XtaskError> {
        let script = repo_root.join("scripts/reset-xplane-sim.sh");
        let status = std::process::Command::new("bash")
            .arg(&script)
            // The script kills THIS checkout's flight controller by
            // path; a checkout resolved through AVIATE_DIR would
            // silently escape a hardcoded pattern and the reset latch
            // would never clear.
            .env("AVIATE_DIR", aviate_dir(repo_root))
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

/// Where the Aviate checkout lives: `AVIATE_DIR`, else `../Aviate` next
/// to this repository. A directory convention, never a source
/// dependency.
fn aviate_dir(repo_root: &Path) -> PathBuf {
    std::env::var_os("AVIATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root.join("../Aviate"))
}

#[cfg(test)]
mod tests;
