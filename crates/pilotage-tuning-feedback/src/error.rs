use std::path::PathBuf;

use thiserror::Error;

/// An error in simulator tuning feedback evidence.
#[derive(Debug, Error)]
pub enum FeedbackError {
    /// Durable evidence storage failed.
    #[error("tuning evidence storage failed: {source}")]
    DurableStorage {
        /// The exact storage failure.
        #[source]
        source: Box<pilotage_durable_storage::StorageError>,
    },
    /// One evidence relation or value is not valid.
    #[error("invalid tuning evidence: {detail}")]
    Invalid {
        /// The exact validation failure.
        detail: String,
    },
    /// JSON encoding failed.
    #[error("cannot encode {document}: {source}")]
    Encode {
        /// The document type.
        document: &'static str,
        /// The JSON failure.
        #[source]
        source: serde_json::Error,
    },
    /// JSON decoding failed.
    #[error("cannot decode {document}: {source}")]
    Decode {
        /// The document type.
        document: &'static str,
        /// The JSON failure.
        #[source]
        source: serde_json::Error,
    },
    /// An evidence file operation failed.
    #[error("cannot {operation} evidence file {path:?}: {source}")]
    FileIo {
        /// The failed operation.
        operation: &'static str,
        /// The evidence file.
        path: PathBuf,
        /// The input or output failure.
        #[source]
        source: std::io::Error,
    },
}

pub(crate) fn invalid(detail: impl Into<String>) -> FeedbackError {
    FeedbackError::Invalid {
        detail: detail.into(),
    }
}
