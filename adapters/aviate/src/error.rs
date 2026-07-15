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
    /// The generated POSIX shared-memory name was not a valid C string.
    #[error("Aviate state shm name {name} is invalid: {source}")]
    ShmName {
        /// The POSIX shm object name.
        name: String,
        /// The embedded-NUL error.
        #[source]
        source: std::ffi::NulError,
    },
    /// A POSIX shared-memory operation failed.
    #[error("Aviate state shm {name} operation {operation} failed: {source}")]
    ShmIo {
        /// The POSIX shm object name.
        name: String,
        /// The operation that failed.
        operation: &'static str,
        /// The operating-system failure.
        #[source]
        source: std::io::Error,
    },
    /// The shared-memory object is too small to back the reader's mapping.
    /// This is a capacity check, not a layout/version check: capacity does
    /// not establish layout compatibility — it proves only that at least
    /// `required` bytes are present, never that the block's fields match.
    #[error("Aviate state shm {name} reports {observed} bytes; needs at least {required}")]
    ShmCapacityTooSmall {
        /// The POSIX shm object name.
        name: String,
        /// Byte count the reader must be able to map.
        required: usize,
        /// The kernel-reported object size (`st_size`), which may be negative.
        observed: i64,
    },
}
