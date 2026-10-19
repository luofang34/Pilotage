//! Typed errors for the xtask launcher.

use std::path::PathBuf;

/// Why an xtask command could not run or has stopped.
#[derive(Debug, thiserror::Error)]
pub enum XtaskError {
    /// Argument parsing failed: an unknown flag or missing/malformed value.
    #[error("usage error: {message}")]
    Usage {
        /// What was wrong with the arguments.
        message: String,
    },
    /// At least one guard failed its self-test or its check. Every
    /// failing guard is named, so one red run reports them all.
    #[error("guards failed: {}", names.join(", "))]
    GuardsFailed {
        /// Every failing guard, in discovery order.
        names: Vec<String>,
    },
    /// Guard discovery found no pairs at all. An empty result means
    /// the runner is looking at the wrong tree, not at a clean one.
    #[error("no guard pairs were discovered under {}", scripts.display())]
    NoGuardPairs {
        /// The directory discovery searched.
        scripts: PathBuf,
    },
    /// `--fc` named a backend this launcher does not know. Unknown
    /// backends fail closed instead of guessing.
    #[error("unknown --fc backend {name:?} (expected: aviate)")]
    UnknownBackend {
        /// The rejected backend name.
        name: String,
    },
    /// `--fc <backend>:<model>` named a model the backend does not launch.
    #[error("the {backend} backend has no model {model:?} (models: {})", if known.is_empty() { "none".to_owned() } else { known.join(", ") })]
    UnknownModel {
        /// The backend asked for the model.
        backend: &'static str,
        /// The rejected model name.
        model: String,
        /// Every model the backend does launch.
        known: Vec<&'static str>,
    },
    /// A fleet asked a backend to place its vehicle, and the backend
    /// cannot. Refused before any session starts.
    #[error("the {backend} backend cannot place its vehicle at a fleet spawn point")]
    SpawnUnsupported {
        /// The backend that has no spawn hook.
        backend: &'static str,
    },
    /// Two fleet vehicles claim the same port or singleton.
    #[error("fleet vehicles {first} and {second} both need {claim}")]
    FleetClaimOverlap {
        /// The earlier vehicle.
        first: String,
        /// The later vehicle.
        second: String,
        /// What both claim.
        claim: String,
    },
    /// A world file has no single origin to move.
    #[error("{} has no single <{tag}> to move the world origin with", world.display())]
    WorldOrigin {
        /// The world file.
        world: PathBuf,
        /// The tag found zero or several times.
        tag: &'static str,
    },
    /// A gz server config cannot be set up for a session.
    #[error("{}: {reason}", path.display())]
    ServerConfig {
        /// The server config file.
        path: PathBuf,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// A reset named an instance no live xtask supervisor runs: the reset
    /// would stop an FC that nothing starts again.
    #[error(
        "no xtask supervisor runs simulator instance {instance} of this checkout; nothing would restart its FC"
    )]
    Unsupervised {
        /// The instance asked for.
        instance: u16,
    },
    /// A backend that resets only instance 0 was asked for another.
    #[error("the {backend} backend resets only instance 0 (asked for {instance})")]
    ResetUnsupported {
        /// The backend.
        backend: &'static str,
        /// The instance asked for.
        instance: u16,
    },
    /// `--profile` named an unknown session profile. Mirrors the host's
    /// own fail-closed parsing: a typo must never fail open.
    #[error("unknown --profile {value:?} (expected physical, simulation, or oracle-only)")]
    UnknownProfile {
        /// The rejected profile value.
        value: String,
    },
    /// A tool or artifact a backend needs is absent.
    #[error("{what} not found at {}: {hint}", path.display())]
    MissingArtifact {
        /// What was being looked for.
        what: &'static str,
        /// Where it was expected.
        path: PathBuf,
        /// How to produce it.
        hint: &'static str,
    },
    /// Something the session must hold is already held: a port is bound
    /// or a singleton process runs. The launcher never stops processes it
    /// did not start, so it names each one and refuses.
    #[error(
        "the session conflicts with what is already running:\n{}",
        conflicts.join("\n")
    )]
    SessionConflict {
        /// One line per held claim.
        conflicts: Vec<String>,
    },
    /// Spawning a managed process failed.
    #[error("spawning {name} failed: {source}")]
    Spawn {
        /// The stage that failed to start.
        name: &'static str,
        /// The underlying OS error.
        #[source]
        source: std::io::Error,
    },
    /// A stage did not become ready before its deadline.
    #[error("{name} was not ready within {seconds} s ({detail}); log: {}", log.display())]
    NotReady {
        /// The stage that never became ready.
        name: &'static str,
        /// The deadline that elapsed.
        seconds: u64,
        /// What readiness was being waited for.
        detail: String,
        /// Where the stage's output was captured.
        log: PathBuf,
    },
    /// A stage exited while the session was starting or running.
    #[error("{name} exited ({status}); last log lines:\n{tail}")]
    StageDied {
        /// The stage that died.
        name: &'static str,
        /// Its exit status.
        status: String,
        /// The tail of its captured log.
        tail: String,
    },
    /// A build or helper command reported failure.
    #[error("{context} failed with {status}")]
    CommandFailed {
        /// What was being run.
        context: &'static str,
        /// The reported exit status.
        status: String,
    },
    /// A running simulator did not prove a required live capability.
    #[error("simulator capability {capability} is unavailable: {detail}")]
    SimulatorCapability {
        /// The capability that the backend requires before FC start.
        capability: &'static str,
        /// The observed refusal or protocol error.
        detail: String,
    },
    /// A site cannot place a vehicle: its position is not well formed.
    #[error("site {site:?} cannot place a vehicle: {source}")]
    SitePosition {
        /// The site's name.
        site: &'static str,
        /// Why the world model refused the position.
        #[source]
        source: pilotage_world::PositionError,
    },
    /// An I/O operation outside process management failed.
    #[error("I/O failure during {context}: {source}")]
    Io {
        /// What was being done.
        context: &'static str,
        /// The underlying OS error.
        #[source]
        source: std::io::Error,
    },
    /// The user pressed ctrl-c before the session was ready; every
    /// started stage has been torn down. A requested stop, not a fault.
    #[error("session stopped before it was ready")]
    Cancelled,
    /// The host proved it is listening on a different port than the one
    /// requested: advertising the requested port would hand out a URL
    /// that cannot connect.
    #[error("host is listening on port {actual}, not the requested {requested}")]
    PortMismatch {
        /// The port the launcher asked for.
        requested: u16,
        /// The port the host's LISTENING line proved.
        actual: u16,
    },
}
