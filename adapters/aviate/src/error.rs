//! Error type for the Aviate adapter.

/// Why the Aviate adapter could not start or has stopped.
#[derive(Debug, thiserror::Error)]
pub enum AviateAdapterError {
    /// Neither the direct MAVLink port nor an ephemeral fallback socket
    /// could be bound.
    #[error("binding a UDP socket for MAVLink telemetry failed: {source}")]
    Bind {
        /// The underlying socket error.
        #[source]
        source: std::io::Error,
    },
}
