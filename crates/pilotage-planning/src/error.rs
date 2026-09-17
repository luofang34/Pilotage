//! Errors with the affected planning record or database path.

use std::path::PathBuf;

/// A planning operation could not finish.
#[derive(Debug, thiserror::Error)]
pub enum PlanningError {
    /// A record violates the planning contract.
    #[error("invalid {field}: {reason}")]
    Invalid {
        /// The affected field or record.
        field: String,
        /// The failed requirement.
        reason: String,
    },
    /// A database operation failed.
    #[error("navigation database {path}: {source}")]
    Database {
        /// The affected database.
        path: PathBuf,
        /// The database failure.
        #[source]
        source: rusqlite::Error,
    },
    /// A planning record could not be encoded or decoded.
    #[error("planning document: {0}")]
    Json(#[from] serde_json::Error),
    /// A navigation snapshot could not be decoded.
    #[error("navigation snapshot: {0}")]
    Navigation(#[from] aerocontext_navdata::BlobError),
    /// A file operation failed.
    #[error("planning file {path}: {source}")]
    File {
        /// The affected file.
        path: PathBuf,
        /// The file failure.
        #[source]
        source: std::io::Error,
    },
}

pub(crate) fn invalid(field: impl Into<String>, reason: impl Into<String>) -> PlanningError {
    PlanningError::Invalid {
        field: field.into(),
        reason: reason.into(),
    }
}
