use std::path::PathBuf;

/// A package operation failed.
#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    /// Another writer owns the package store.
    #[error("package store at {path} already has a writer")]
    StoreBusy {
        /// Store directory.
        path: PathBuf,
    },
    /// The navigation snapshot could not be decoded.
    #[error("navigation snapshot could not be decoded at {path}")]
    Navigation {
        /// Affected snapshot path.
        path: PathBuf,
        /// Navigation format error.
        #[source]
        source: aerocontext_navdata::BlobError,
    },
    /// A manifest field violates its contract.
    #[error("invalid package field {field}: {value}")]
    Invalid {
        /// Field name.
        field: &'static str,
        /// Rejected value or reason.
        value: String,
    },
    /// A JSON document could not be decoded or encoded.
    #[error("package JSON could not be processed")]
    Json(#[from] serde_json::Error),
    /// A file operation failed.
    #[error("package file operation failed at {path}")]
    Io {
        /// Affected path.
        path: PathBuf,
        /// File system error.
        #[source]
        source: std::io::Error,
    },
    /// An installation database operation failed.
    #[error("package database operation failed at {path}")]
    Database {
        /// Database path.
        path: PathBuf,
        /// SQLite error.
        #[source]
        source: rusqlite::Error,
    },
    /// Downloaded bytes do not match the manifest.
    #[error("object {expected} has digest {actual}")]
    Digest {
        /// Required digest.
        expected: String,
        /// Observed digest.
        actual: String,
    },
    /// A download has an incorrect length.
    #[error("object {digest} requires {expected} bytes, received {actual}")]
    Length {
        /// Object digest.
        digest: String,
        /// Required length.
        expected: u64,
        /// Observed length.
        actual: u64,
    },
    /// The caller has not reserved enough storage.
    #[error("package installation needs {required} bytes; {available} bytes are available")]
    Space {
        /// Required additional storage.
        required: u64,
        /// Available storage.
        available: u64,
    },
    /// A release or dependency is not installed.
    #[error("package {id} is not installed")]
    Missing {
        /// Required release ID.
        id: String,
    },
    /// An immutable ID already names different content.
    #[error("package {id} already identifies different content")]
    IdentityConflict {
        /// Reused release ID.
        id: String,
    },
    /// A selected release is still required.
    #[error("package {id} is retained by {owner}")]
    Retained {
        /// Release ID.
        id: String,
        /// Selection or dependent release.
        owner: String,
    },
    /// Catalog signature verification failed.
    #[error("catalog signature failed for key {key_id}")]
    Signature {
        /// Claimed signing key.
        key_id: String,
        /// Signature error.
        #[source]
        source: ed25519_dalek::SignatureError,
    },
}

pub(crate) fn invalid(field: &'static str, value: impl ToString) -> PackageError {
    PackageError::Invalid {
        field,
        value: value.to_string(),
    }
}

pub(crate) fn file_error(path: impl Into<PathBuf>, source: std::io::Error) -> PackageError {
    PackageError::Io {
        path: path.into(),
        source,
    }
}
