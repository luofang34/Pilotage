use std::path::PathBuf;

/// A local map resource could not be resolved.
#[derive(Debug, thiserror::Error)]
pub enum ResourceError {
    /// A binding has an invalid URI or a relative file path.
    #[error("invalid local resource binding {uri} at {path}")]
    Binding {
        /// Rejected resource URI.
        uri: String,
        /// Rejected file path.
        path: PathBuf,
    },
    /// Two files use the same resource URI.
    #[error("duplicate local resource URI {uri}")]
    Duplicate {
        /// Repeated URI.
        uri: String,
    },
    /// The request does not use a supported resource URI.
    #[error("invalid local resource request {uri}")]
    Uri {
        /// Rejected request URI.
        uri: String,
    },
    /// The map has no binding for this resource.
    #[error("local resource is not bound: {uri}")]
    Unknown {
        /// Requested URI.
        uri: String,
    },
    /// An archive read failed.
    #[error(transparent)]
    Archive(#[from] crate::ArchiveError),
    /// A file read failed.
    #[error("cannot read local map resource at {path}")]
    Io {
        /// File path.
        path: PathBuf,
        /// File error.
        #[source]
        source: std::io::Error,
    },
    /// A file or decoded tile exceeds the resource limit.
    #[error("local map resource at {path} exceeds {limit} bytes")]
    Size {
        /// File path.
        path: PathBuf,
        /// Maximum resource size in bytes.
        limit: u64,
    },
    /// A blocking archive reader could not finish.
    #[error("local map archive worker failed at {path}")]
    Worker {
        /// Archive path.
        path: PathBuf,
        /// Worker error.
        #[source]
        source: tokio::task::JoinError,
    },
}
