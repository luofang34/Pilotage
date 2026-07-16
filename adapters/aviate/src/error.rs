//! Error type for the Aviate adapter.

/// Why the Aviate adapter could not start or has stopped.
#[derive(Debug, thiserror::Error)]
pub enum AviateAdapterError {
    /// The attachment identity provider could not obtain a new incarnation.
    #[error("creating an Aviate source incarnation failed: {source}")]
    IncarnationUnavailable {
        /// The operating-system entropy failure.
        #[source]
        source: getrandom::Error,
    },
    /// Neither the direct MAVLink port nor an ephemeral fallback socket
    /// could be bound.
    #[error("binding a UDP socket for MAVLink telemetry failed: {source}")]
    Bind {
        /// The underlying socket error.
        #[source]
        source: std::io::Error,
    },
    /// Attaching to the XIL shm object failed at the operating-system
    /// level: the object does not exist (no simulation writer is up) or
    /// the kernel refused the read-only mapping.
    #[error("Aviate XIL shm {name} attach failed: {source}")]
    ShmAttachIo {
        /// The POSIX shm object name.
        name: String,
        /// The operating-system failure reported by the upstream attach.
        #[source]
        source: std::io::Error,
    },
    /// The object exists but is not the contract block this build was
    /// compiled against (foreign magic, layout version, declared size, or
    /// a truncated mapping). Reading it would be plausible garbage, so the
    /// attachment fails closed.
    #[error("Aviate XIL shm {name} contract violation: {violation:?}")]
    ShmContractMismatch {
        /// The POSIX shm object name.
        name: String,
        /// The upstream attach-rule violation, verbatim.
        violation: aviate_xil_contract::AttachError,
    },
    /// The block validates but the simulation writer has not published
    /// readiness yet (writer mid-initialization). Retryable: attach again
    /// once the writer is up; payload fields must not be read before then.
    #[error("Aviate XIL shm {name} writer not ready")]
    ShmWriterNotReady {
        /// The POSIX shm object name.
        name: String,
    },
}
