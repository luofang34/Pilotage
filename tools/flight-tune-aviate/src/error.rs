use std::path::PathBuf;
use std::process::ExitStatus;

/// A supervised Aviate process operation failed.
#[derive(Debug, thiserror::Error)]
pub enum AviateSupervisorError {
    /// The selected platform cannot provide the required process identity.
    #[error("Aviate process supervision is not supported on this platform")]
    UnsupportedPlatform,
    /// A supplied process request is incomplete or unsafe.
    #[error("invalid Aviate process supervision request: {detail}")]
    InvalidRequest {
        /// Stable validation detail.
        detail: String,
    },
    /// Anchored durable storage failed.
    #[error("Aviate process supervision storage failed during {operation}: {source}")]
    Storage {
        /// Storage operation that failed.
        operation: &'static str,
        /// Exact storage failure.
        #[source]
        source: Box<pilotage_durable_storage::StorageError>,
    },
    /// A file-system operation failed.
    #[error("Aviate process supervision {operation} failed for {path:?}: {source}")]
    Io {
        /// Operation that failed.
        operation: &'static str,
        /// Exact path selected by the operation.
        path: PathBuf,
        /// Operating-system failure.
        #[source]
        source: std::io::Error,
    },
    /// A process operation failed.
    #[error("Aviate process supervision {operation} failed: {source}")]
    ProcessIo {
        /// Process operation that failed.
        operation: &'static str,
        /// Operating-system failure.
        #[source]
        source: std::io::Error,
    },
    /// A durable process document is invalid.
    #[error("invalid Aviate supervisor {document}: {detail}")]
    InvalidDocument {
        /// Document class.
        document: &'static str,
        /// Stable validation detail.
        detail: String,
        /// JSON failure that caused the invalid document, when present.
        #[source]
        source: Option<serde_json::Error>,
    },
    /// A live process differs from its durable identity.
    #[error("Aviate process identity mismatch: {detail}")]
    IdentityMismatch {
        /// Stable mismatch detail.
        detail: String,
    },
    /// The supervisor protocol rejected a message.
    #[error("invalid Aviate supervisor protocol: {detail}")]
    Protocol {
        /// Stable protocol detail.
        detail: String,
        /// JSON failure that caused the protocol error, when present.
        #[source]
        source: Option<serde_json::Error>,
    },
    /// A bounded process operation did not finish.
    #[error("Aviate process supervision timed out during {operation}")]
    Timeout {
        /// Bounded operation that timed out.
        operation: &'static str,
    },
    /// The supervisor stopped before it completed the requested operation.
    #[error("Aviate process supervisor stopped with {status}")]
    SupervisorExited {
        /// Supervisor exit status.
        status: ExitStatus,
    },
    /// Another live supervisor holds the exact storage writer lease.
    #[error("another Aviate process supervisor holds the writer lease")]
    SupervisorActive,
    /// Recovery found a process or process group that it cannot safely classify.
    #[error("Aviate process recovery stopped: {detail}")]
    RecoveryBlocked {
        /// Stable fail-closed detail.
        detail: String,
    },
    /// A Darwin system-control operation failed.
    #[cfg(target_os = "macos")]
    #[error("Aviate process supervision Darwin {operation} failed: {source}")]
    DarwinSystemControl {
        /// System-control operation.
        operation: &'static str,
        /// Darwin system-control failure.
        #[source]
        source: sysctl::SysctlError,
    },
    /// Startup failed and the owner could not report the exact rejection.
    #[error("Aviate process startup failed: {source}; report to parent failed: {notification}")]
    ReleaseNotification {
        /// Startup failure.
        #[source]
        source: Box<AviateSupervisorError>,
        /// Parent-notification failure.
        notification: Box<AviateSupervisorError>,
    },
    /// Startup failed and bounded cleanup also failed.
    #[error("Aviate process startup failed: {source}; cleanup also failed: {cleanup}")]
    StartupCleanup {
        /// Startup failure.
        #[source]
        source: Box<AviateSupervisorError>,
        /// Cleanup failure.
        cleanup: Box<AviateSupervisorError>,
    },
}

impl AviateSupervisorError {
    pub(crate) fn invalid_request(detail: impl Into<String>) -> Self {
        Self::InvalidRequest {
            detail: detail.into(),
        }
    }

    pub(crate) fn invalid_document(document: &'static str, detail: impl Into<String>) -> Self {
        Self::InvalidDocument {
            document,
            detail: detail.into(),
            source: None,
        }
    }

    pub(crate) fn invalid_json_document(
        document: &'static str,
        operation: &'static str,
        source: serde_json::Error,
    ) -> Self {
        Self::InvalidDocument {
            document,
            detail: format!("JSON {operation} failed"),
            source: Some(source),
        }
    }

    pub(crate) fn identity_mismatch(detail: impl Into<String>) -> Self {
        Self::IdentityMismatch {
            detail: detail.into(),
        }
    }

    pub(crate) fn protocol(detail: impl Into<String>) -> Self {
        Self::Protocol {
            detail: detail.into(),
            source: None,
        }
    }

    pub(crate) fn json_protocol(operation: &'static str, source: serde_json::Error) -> Self {
        Self::Protocol {
            detail: format!("JSON {operation} failed"),
            source: Some(source),
        }
    }
}
