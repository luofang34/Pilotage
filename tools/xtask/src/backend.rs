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
    /// Repository-relative control-feel law for the vehicle this backend
    /// launches, or `None` for one whose host does not read a law.
    ///
    /// DECLARED rather than written into [`Self::host_env`], so that
    /// naming a law and deciding whether to pass it are separate jobs. The
    /// decision has two cases that do harm rather than nothing — a
    /// physical session, and an operator who already named one — and a
    /// backend that assembled the variable itself would have to remember
    /// both. Here it cannot: it says which law is its vehicle's, and
    /// [`control_feel_env`] is the only thing that writes the variable.
    fn control_feel_profile(&self) -> Option<&'static str> {
        None
    }
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

/// The variable the host reads a control-feel law from.
///
/// A bare string on this side and another on the host's
/// (`runtime/adapter_launch.rs`); the two crates share no dependency that
/// could hold one constant, so renaming either alone leaves this silently
/// naming nothing.
const CONTROL_FEEL_ENV: &str = "PILOTAGE_AVIATE_CONTROL_FEEL_PROFILE";

/// Names the control-feel law a simulated vehicle flies on.
///
/// The host cannot choose this for itself: every Aviate backend hands it
/// `--adapter aviate`, so absent a name it loads one compiled-in default
/// for all of them. The launcher is the only component that knows which
/// aircraft it started.
///
/// Yields NOTHING in two cases, both of which would do harm rather than
/// nothing:
///
/// * An operator who already names a law keeps it. Stage environment is
///   applied ON TOP of what the launcher inherited, so a value written
///   here replaces theirs with no diagnostic anywhere — the vehicle would
///   fly a law nobody chose and nothing would say so. A tuning session
///   pointing at its own artifact is exactly this path.
/// * A physical session gets no name at all. The host refuses one
///   outright there — `AviatePhysicalControlFeelOverride` — because a real
///   aircraft must fly the qualified compiled-in artifact rather than one
///   a launcher pointed at, so naming one does not go unused, it fails the
///   session.
///
/// Both live here rather than in each backend so a backend added later
/// cannot reintroduce either by writing the variable itself.
pub(crate) fn control_feel_env(ctx: &SessionContext, profile_path: &str) -> Vec<(String, String)> {
    control_feel_env_given(ctx, profile_path, std::env::var_os(CONTROL_FEEL_ENV))
}

/// The decision itself, with the operator's value passed in.
///
/// Reading the environment is the caller's job so this stays a function of
/// its arguments: the alternative is a test that sets a process-wide
/// variable, and the workspace forbids the `unsafe` that now requires.
fn control_feel_env_given(
    ctx: &SessionContext,
    profile_path: &str,
    ambient: Option<std::ffi::OsString>,
) -> Vec<(String, String)> {
    if ctx.profile == Profile::Physical || ambient.is_some() {
        return Vec::new();
    }
    vec![(
        CONTROL_FEEL_ENV.to_owned(),
        ctx.repo_root.join(profile_path).display().to_string(),
    )]
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
    /// `--adapter aviate`, so absent an explicit name it loads one
    /// compiled-in default for every vehicle — a law qualified for a
    /// different airframe. The launcher is the only component that knows
    /// which aircraft it started, so this is where the binding is asserted.
    #[test]
    fn each_aviate_backend_names_its_own_vehicles_control_feel() {
        use std::path::{Path, PathBuf};

        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root is two levels above tools/xtask")
            .to_path_buf();

        // Backend name -> the substring its law's file name must carry.
        for (backend, vehicle) in [("aviate-gz", "x500"), ("aviate-xplane", "alia250")] {
            let declared = backend_for(backend)
                .expect("known backend")
                .control_feel_profile()
                .unwrap_or_else(|| {
                    panic!(
                        "{backend} names no control-feel law, so the host falls back to \
                         whatever single default it was compiled with"
                    )
                });

            let path = repo_root.join(declared);
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

    /// No backend may write the control-feel variable itself.
    ///
    /// A backend that assembles it by hand has to remember every case
    /// where passing a law does harm — a physical session, and an operator
    /// who already named one — and one that remembers only the first
    /// silently replaces the operator's law while every other test here
    /// still passes. Declaring the law and writing the variable are
    /// therefore separate: backends return a path, and the launcher is the
    /// only writer.
    #[test]
    fn no_backend_writes_the_control_feel_variable_itself() {
        use super::{CONTROL_FEEL_ENV, SessionContext};
        use crate::cli::Profile;
        use std::path::PathBuf;

        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        for backend in ["aviate-gz", "aviate-xplane", "px4-gz", "px4-xplane"] {
            for profile in [Profile::Simulation, Profile::Physical, Profile::OracleOnly] {
                let ctx = SessionContext {
                    repo_root: repo_root.clone(),
                    host_port: 4433,
                    viewer_port: 8080,
                    profile,
                    log_dir: repo_root.join("target/xtask-sim"),
                    lan: false,
                };
                let env = backend_for(backend).expect("known backend").host_env(&ctx);
                assert!(
                    !env.iter().any(|(key, _)| key == CONTROL_FEEL_ENV),
                    "{backend} writes {CONTROL_FEEL_ENV} in host_env for {profile:?}; \
                     it must declare the law through control_feel_profile() and let the \
                     launcher decide whether passing it is safe"
                );
            }
        }
    }

    /// A physical session must be handed NO control-feel law.
    ///
    /// The host does not ignore one there, it refuses the session
    /// outright, so a launcher that names a law unconditionally turns a
    /// working `--profile physical` run into a startup failure. Naming a
    /// law per vehicle is what creates that risk, and this test is what
    /// holds it closed.
    #[test]
    fn a_physical_session_is_handed_no_control_feel_law() {
        use super::SessionContext;
        use crate::cli::Profile;
        use std::path::PathBuf;

        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        for backend in ["aviate-gz", "aviate-xplane"] {
            let ctx = SessionContext {
                repo_root: repo_root.clone(),
                host_port: 4433,
                viewer_port: 8080,
                profile: Profile::Physical,
                log_dir: repo_root.join("target/xtask-sim"),
                lan: false,
            };
            let declared = backend_for(backend)
                .expect("known backend")
                .control_feel_profile()
                .expect("an Aviate backend declares a law");
            assert!(
                super::control_feel_env(&ctx, declared).is_empty(),
                "{backend} names a control-feel law for a physical session, which the \
                 host refuses with AviatePhysicalControlFeelOverride"
            );
        }
    }

    /// An operator who already names a law keeps it.
    ///
    /// Stage environment is applied on top of what the launcher inherited,
    /// so a value written by a backend REPLACES the operator's with no
    /// diagnostic on either side: the vehicle flies a law nobody chose and
    /// nothing says so. That is the same failure this whole change exists
    /// to prevent, pointed the other way.
    #[test]
    fn an_operators_own_control_feel_law_is_not_replaced() {
        use super::{CONTROL_FEEL_ENV, SessionContext, control_feel_env_given};
        use crate::cli::Profile;
        use std::path::PathBuf;

        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let ctx = SessionContext {
            repo_root: repo_root.clone(),
            host_port: 4433,
            viewer_port: 8080,
            profile: Profile::Simulation,
            log_dir: repo_root.join("target/xtask-sim"),
            lan: false,
        };

        let operators = std::ffi::OsString::from("/tmp/an-operators-own-tuning.json");
        assert!(
            control_feel_env_given(
                &ctx,
                "adapters/aviate/profiles/x500-shaped-balanced-v1.json",
                Some(operators)
            )
            .is_empty(),
            "the launcher writes {CONTROL_FEEL_ENV} over the operator's own value, which \
             the child inherits and would otherwise have used"
        );

        // The same call with nothing ambient must still bind, or the guard
        // above would be satisfied by never naming a law at all.
        assert_eq!(
            control_feel_env_given(
                &ctx,
                "adapters/aviate/profiles/x500-shaped-balanced-v1.json",
                None
            )
            .len(),
            1,
            "with no operator value the launcher must name the vehicle's law"
        );
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
