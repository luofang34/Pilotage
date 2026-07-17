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
    /// `--fc` named a backend this launcher does not know. Unknown
    /// backends fail closed instead of guessing.
    #[error("unknown --fc backend {name:?} (expected: aviate)")]
    UnknownBackend {
        /// The rejected backend name.
        name: String,
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
    /// Session processes from a previous run are still alive. The
    /// launcher never kills processes it did not start.
    #[error(
        "session processes are already running:\n{listing}\nstop them first: pkill -f \"gz sim\"; pkill -f sitl-gazebo-x500; pkill -f pilotage-session-host; pkill -f \"http.server\""
    )]
    StaleSession {
        /// `pgrep` listing of the offending processes.
        listing: String,
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
    /// An I/O operation outside process management failed.
    #[error("I/O failure during {context}: {source}")]
    Io {
        /// What was being done.
        context: &'static str,
        /// The underlying OS error.
        #[source]
        source: std::io::Error,
    },
}
