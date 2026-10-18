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
pub(crate) mod gz;
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
    /// Directory stage logs are written under. Each session has its own,
    /// so concurrent sessions never write one another's logs.
    pub log_dir: PathBuf,
    /// Directory the viewer serves: the session's connect manifest
    /// (`session.json`) is written here, so each viewer hands a stale tab
    /// its own session and never another one's.
    pub web_root: PathBuf,
    /// Serve the session to the local network rather than loopback only.
    pub lan: bool,
    /// Simulator instance index. Instance 0 is the single-session default;
    /// a backend that can run several copies at once shifts its ports and
    /// isolates its simulator by this index.
    pub instance: u16,
}

#[cfg(test)]
impl SessionContext {
    /// The single-session defaults (instance 0, ports 4433/8080, logs and
    /// web root under `repo_root`), for tests to adjust field by field.
    pub(crate) fn for_test(repo_root: PathBuf) -> Self {
        Self {
            log_dir: repo_root.join("target/xtask-sim"),
            web_root: repo_root.join("clients/web"),
            repo_root,
            host_port: 4433,
            viewer_port: 8080,
            profile: Profile::Simulation,
            lan: false,
            instance: 0,
        }
    }
}

/// Where a fleet puts one vehicle: a point on the WGS84 ellipsoid. The
/// height is the backend's own ground, so each vehicle starts on the
/// field its world models.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpawnPoint {
    /// Latitude, degrees north.
    pub latitude_deg: f64,
    /// Longitude, degrees east.
    pub longitude_deg: f64,
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
    /// Puts this backend's vehicle at `point` by editing its planned
    /// stages: a world whose origin moves, a home argument, a sidecar
    /// option — whatever the simulator takes.
    ///
    /// A fleet calls this for every vehicle before it starts anything, so
    /// a backend that cannot place its vehicle is refused up front rather
    /// than stacked on top of another vehicle at the same point.
    ///
    /// # Errors
    ///
    /// Returns [`XtaskError::SpawnUnsupported`] by default, or the
    /// backend's own failure to place the vehicle.
    fn spawn(
        &self,
        _ctx: &SessionContext,
        _point: &SpawnPoint,
        _stages: &mut [Stage],
    ) -> Result<(), XtaskError> {
        Err(XtaskError::SpawnUnsupported {
            backend: self.name(),
        })
    }
    /// Selects the vehicle model named after the backend (`--fc
    /// <backend>:<model>`). A backend with one vehicle has no models.
    ///
    /// # Errors
    ///
    /// Returns [`XtaskError::UnknownModel`] for a model this backend does
    /// not launch.
    fn select_model(&mut self, model: &str) -> Result<(), XtaskError> {
        Err(XtaskError::UnknownModel {
            backend: self.name(),
            model: model.to_owned(),
            known: Vec::new(),
        })
    }
    /// `pgrep -f` patterns that mark a stale session of this backend.
    fn stale_process_patterns(&self) -> Vec<&'static str>;
    /// What this session holds that another session must not: the ports
    /// its simulator and FC bind, and any machine-wide singleton.
    ///
    /// The launcher refuses to start only when one of these is in use, so
    /// sessions that claim different things run side by side. The default
    /// treats every process that matches [`Self::stale_process_patterns`]
    /// as a conflict: a backend that does not say what it holds runs one
    /// session per machine, never two that fight over a resource.
    fn claims(&self, _ctx: &SessionContext) -> Vec<crate::conflicts::Claim> {
        self.stale_process_patterns()
            .into_iter()
            .map(crate::conflicts::Claim::Process)
            .collect()
    }
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

/// The host environment a session actually hands the host stage.
///
/// The backend's own entries plus the control-feel law it declares, which
/// only this composition decides to pass. It exists as a function so the
/// binding can be asserted against the value the launcher builds rather
/// than against either half of it: a launcher that assembles this and
/// then hands the host something else is the whole defect back, and two
/// separately-correct halves do not rule that out.
pub(crate) fn host_env_for(
    backend: &dyn SimBackend,
    ctx: &SessionContext,
) -> Vec<(String, String)> {
    let mut env = backend.host_env(ctx);
    if let Some(profile_path) = backend.control_feel_profile() {
        env.extend(control_feel_env(ctx, profile_path));
    }
    env
}

/// Every backend `--fc` resolves to.
///
/// A table rather than a match arm per backend, because two things must
/// agree about what "every backend" means: this resolver, and the tests
/// that hold every backend to a contract. A backend reachable from one
/// but absent from the other is a backend nothing checks — and the
/// direction that fails is the safe one, since a backend missing here
/// cannot be selected at all.
const BACKENDS: &[BackendEntry] = &[
    // Canonical names pair the FC family with the simulator behind it;
    // the bare FC name stays accepted as the family's default.
    BackendEntry {
        name: "aviate-gz",
        alias: Some("aviate"),
        make: || Box::new(aviate_gz::AviateGz),
    },
    BackendEntry {
        name: "px4-gz",
        alias: Some("px4"),
        make: || Box::new(px4_gz::Px4Gz::default()),
    },
    BackendEntry {
        name: "px4-xplane",
        alias: None,
        make: || Box::new(px4_xplane::Px4XPlane),
    },
    BackendEntry {
        name: "aviate-xplane",
        alias: None,
        make: || Box::new(aviate_xplane::AviateXPlane),
    },
];

struct BackendEntry {
    name: &'static str,
    alias: Option<&'static str>,
    make: fn() -> Box<dyn SimBackend>,
}

/// Where the sibling Aviate checkout lives: `AVIATE_DIR`, else
/// `../Aviate` next to this repository. A directory convention, never a
/// source dependency.
fn aviate_dir(repo_root: &std::path::Path) -> PathBuf {
    std::env::var_os("AVIATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root.join("../Aviate"))
}

/// The Alia X-Plane simulator-model preset inside the Aviate checkout —
/// the one document every handshake producer must agree on.
fn alia_xplane_preset(repo_root: &std::path::Path) -> PathBuf {
    aviate_dir(repo_root).join("presets/alia250-xplane.toml")
}

/// Produces the Alia X-Plane runtime handshake without launching a
/// session, for harnesses that run the flight controller themselves.
/// Blocks until the trial plugin inside X-Plane states its own identity,
/// exactly as a session launch does — the run is bound to the simulator
/// that is actually running, not to one a caller claimed was.
///
/// # Errors
///
/// Returns a typed [`XtaskError`] when X-Plane is not reachable, the
/// plugins do not verify, or the handshake cannot be written.
pub(crate) fn produce_xplane_handshake_blocking(
    repo_root: &std::path::Path,
    out_dir: &std::path::Path,
) -> Result<PathBuf, XtaskError> {
    let root = xplane_simulator::xplane_root()?;
    xplane_handshake::produce_blocking(&root, &alia_xplane_preset(repo_root), out_dir)
}

/// Resolves `--fc <backend>[:<model>]` to a backend, fail-closed on
/// unknown names and models.
///
/// # Errors
///
/// Returns [`XtaskError::UnknownBackend`] for any name this launcher
/// does not implement, and [`XtaskError::UnknownModel`] for a model the
/// backend does not launch.
pub fn backend_for(spec: &str) -> Result<Box<dyn SimBackend>, XtaskError> {
    let (name, model) = match spec.split_once(':') {
        Some((name, model)) => (name, Some(model)),
        None => (spec, None),
    };
    let mut backend = BACKENDS
        .iter()
        .find(|entry| entry.name == name || entry.alias == Some(name))
        .map(|entry| (entry.make)())
        .ok_or_else(|| XtaskError::UnknownBackend {
            name: name.to_owned(),
        })?;
    if let Some(model) = model {
        backend.select_model(model)?;
    }
    Ok(backend)
}

#[cfg(test)]
mod tests;
