//! The FC-backend seam: each backend plans its simulator/FC stages and
//! contributes the host's adapter and environment. The launcher itself
//! stays backend-agnostic so future FC families (PX4, JSBSim) are new
//! implementations, not new orchestration.

use std::path::PathBuf;

use crate::cli::Profile;
use crate::error::XtaskError;
use crate::process::ProcessSpec;
use crate::readiness::Readiness;

pub(crate) mod aviate_gz;
mod aviate_xplane;
pub(crate) mod px4_gz;
mod px4_xplane;
mod xplane_handshake;
mod xplane_simulator;

/// Everything a backend may need to plan its stages.
#[derive(Debug)]
pub struct SessionContext {
    /// Workspace root of this repository.
    pub repo_root: PathBuf,
    /// Host WebTransport port.
    pub host_port: u16,
    /// Static viewer port.
    pub viewer_port: u16,
    /// Session profile handed to the host.
    pub profile: Profile,
    /// Directory stage logs are written under.
    pub log_dir: PathBuf,
    /// Serve the session to the local network rather than loopback only.
    pub lan: bool,
}

/// One plannable launch step: a process and the signal proving it is up.
#[derive(Debug)]
pub struct Stage {
    /// The process to run.
    pub spec: ProcessSpec,
    /// The readiness signal to wait for before the next stage.
    pub readiness: Readiness,
}

/// A launchable FC/simulator family.
pub trait SimBackend {
    /// Backend name as selected by `--fc`.
    fn name(&self) -> &'static str;
    /// The session host `--adapter` this backend's telemetry plane uses.
    fn host_adapter(&self) -> &'static str;
    /// Extra environment the host needs for this backend.
    fn host_env(&self, ctx: &SessionContext) -> Vec<(String, String)>;
    /// Validates tools/artifacts and plans the simulator and FC stages,
    /// in launch order.
    ///
    /// # Errors
    ///
    /// Returns [`XtaskError::MissingArtifact`] with an actionable hint
    /// when a required tool or build product is absent.
    fn plan(&self, ctx: &SessionContext) -> Result<Vec<Stage>, XtaskError>;
    /// Builds this backend's own gitignored artifacts so a fresh checkout
    /// runs out of the box. Best-effort by contract: a backend whose extra
    /// artifact only enriches the session (e.g. camera video that degrades to
    /// no-video) must warn and return `Ok` when its toolchain is absent, so a
    /// missing optional dependency never blocks the flight. The default is a
    /// no-op for backends with nothing extra to build.
    ///
    /// # Errors
    ///
    /// Returns a typed [`XtaskError`] only for a failure that must abort the
    /// session; recoverable/optional build failures are logged, not returned.
    /// Re-establish whatever a restarted stage consumes on start.
    ///
    /// A stage's argv is planned once and reused for every restart, so
    /// anything it CONSUMES rather than reads — a single-use credential, a
    /// claimed file — is gone by the time the replacement runs. This is where
    /// it comes back. Producing it at the same path the plan named keeps the
    /// argv correct.
    ///
    /// Called before the replacement is spawned, never before the first start.
    ///
    /// The work is RETURNED rather than performed, because the caller drives a
    /// current-thread runtime. Blocking here stops that runtime polling, and
    /// the task watching for ctrl-c stops with it — so an operator pressing it
    /// during a long wait gets nothing at all, tokio having already taken the
    /// signal's default disposition away. Handed back as an owned unit, it can
    /// run off the runtime thread and be raced against cancellation.
    ///
    /// `None` when this backend has nothing to re-establish for this stage.
    fn before_stage_restart(
        &self,
        _ctx: &SessionContext,
        _stage_name: &str,
    ) -> Option<Box<dyn FnOnce() -> Result<(), XtaskError> + Send>> {
        None
    }

    fn prepare(&self, ctx: &SessionContext) -> Result<(), XtaskError> {
        let _ = ctx;
        Ok(())
    }
    /// `pgrep -f` patterns that mark a stale session of this backend.
    fn stale_process_patterns(&self) -> Vec<&'static str>;
    /// Resets the running simulation world and FC.
    ///
    /// # Errors
    ///
    /// Returns [`XtaskError::CommandFailed`] when the reset reports
    /// failure.
    fn reset(&self, repo_root: &std::path::Path) -> Result<(), XtaskError>;
}

/// Resolves `--fc` to a backend, fail-closed on unknown names.
///
/// # Errors
///
/// Returns [`XtaskError::UnknownBackend`] for any name this launcher
/// does not implement.
pub fn backend_for(name: &str) -> Result<Box<dyn SimBackend>, XtaskError> {
    match name {
        // Canonical names pair the FC family with the simulator behind
        // it; the bare FC name stays accepted as the family's default.
        "aviate-gz" | "aviate" => Ok(Box::new(aviate_gz::AviateGz)),
        "px4-gz" | "px4" => Ok(Box::new(px4_gz::Px4Gz)),
        "px4-xplane" => Ok(Box::new(px4_xplane::Px4XPlane)),
        "aviate-xplane" => Ok(Box::new(aviate_xplane::AviateXPlane)),
        _ => Err(XtaskError::UnknownBackend {
            name: name.to_owned(),
        }),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::backend_for;
    use crate::error::XtaskError;

    /// Every Aviate backend must name a control-feel law belonging to the
    /// aircraft it actually launches.
    ///
    /// The host cannot check this for itself: both backends hand it
    /// `--adapter aviate`, so absent an explicit profile it loads one
    /// compiled-in default for every vehicle. That default is the Alia's,
    /// and the X500 flew on it — a law qualified for another airframe.
    /// The launcher is the only component that knows which aircraft it
    /// started, so this is where the binding is asserted.
    #[test]
    fn each_aviate_backend_names_its_own_vehicles_control_feel() {
        use super::SessionContext;
        use crate::cli::Profile;
        use std::path::{Path, PathBuf};

        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root is two levels above tools/xtask")
            .to_path_buf();
        let ctx = SessionContext {
            repo_root: repo_root.clone(),
            host_port: 4433,
            viewer_port: 8080,
            profile: Profile::Simulation,
            log_dir: repo_root.join("target/xtask-sim"),
            lan: false,
        };

        // Backend name -> the substring its profile's file name must carry.
        for (backend, vehicle) in [("aviate-gz", "x500"), ("aviate-xplane", "alia250")] {
            let env = backend_for(backend).expect("known backend").host_env(&ctx);
            let (_, value) = env
                .iter()
                .find(|(key, _)| key == "PILOTAGE_AVIATE_CONTROL_FEEL_PROFILE")
                .unwrap_or_else(|| {
                    panic!(
                        "{backend} names no control-feel law, so the host falls back to \
                         whatever single default it was compiled with"
                    )
                });

            let path = PathBuf::from(value);
            let name = path
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or_default();
            assert!(
                name.starts_with(vehicle),
                "{backend} launches {vehicle} but names the control-feel law {name:?}, \
                 which is another aircraft's"
            );
            assert!(
                path.is_file(),
                "{backend} names a control-feel law at {} that does not exist",
                path.display()
            );
        }
    }

    #[test]
    fn backend_selection_fails_closed() {
        assert_eq!(backend_for("aviate").expect("known").name(), "aviate-gz");
        assert_eq!(backend_for("aviate-gz").expect("known").name(), "aviate-gz");
        assert_eq!(backend_for("px4").expect("known").name(), "px4-gz");
        assert_eq!(backend_for("px4-gz").expect("known").name(), "px4-gz");
        assert_eq!(
            backend_for("px4-xplane").expect("known").name(),
            "px4-xplane"
        );
        assert_eq!(
            backend_for("aviate-xplane").expect("known").name(),
            "aviate-xplane"
        );
        let refusal = backend_for("px4-jsbsim");
        assert!(matches!(refusal, Err(XtaskError::UnknownBackend { .. })));
    }
}
