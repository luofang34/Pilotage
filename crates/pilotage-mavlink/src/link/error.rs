//! Why the MAVLink link could not start.

/// Why the MAVLink link could not start.
#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    /// Neither the direct MAVLink port nor an ephemeral fallback socket
    /// could be bound.
    #[error("binding a UDP socket for MAVLink telemetry failed: {source}")]
    Bind {
        /// The underlying socket error.
        #[source]
        source: std::io::Error,
    },
}
