//! Typed errors of the evaluation tool.

use std::path::PathBuf;

/// Every way an evaluation can stop before it has a report.
#[derive(Debug, thiserror::Error)]
pub(crate) enum EvalError {
    /// The command line is not usable.
    #[error("invalid command line: {detail}")]
    Usage {
        /// What is wrong with it.
        detail: String,
    },
    /// The async runtime cannot start.
    #[error("cannot start the async runtime")]
    Runtime(#[source] std::io::Error),
    /// A file cannot be read.
    #[error("cannot read {path}")]
    Read {
        /// The file path.
        path: PathBuf,
        /// The file-system failure.
        #[source]
        source: std::io::Error,
    },
    /// A file cannot be written.
    #[error("cannot write {path}")]
    Write {
        /// The file path.
        path: PathBuf,
        /// The file-system failure.
        #[source]
        source: std::io::Error,
    },
    /// The suite is not a usable document.
    #[error("cannot use suite {path}")]
    Suite {
        /// The suite path.
        path: PathBuf,
        /// What is wrong with it.
        #[source]
        source: pilotage_agent::AgentError,
    },
    /// The model adapter cannot start.
    #[error("the model adapter is not usable")]
    Model(#[source] intent_pilot::ModelProcessError),
}
