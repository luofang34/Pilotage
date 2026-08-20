//! Typed errors for the sidecar video link.

/// Errors the sidecar spawn, connection, and framing paths can produce.
#[derive(Debug, thiserror::Error)]
pub enum SimVideoError {
    /// Binding the localhost TCP listener the sidecar dials back into failed.
    #[error("failed to bind bridge listener on 127.0.0.1:0: {source}")]
    ListenerBind {
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Reading the bound listener's local address failed.
    #[error("failed to read bound bridge listener address: {source}")]
    ListenerAddr {
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Spawning the sidecar child process failed.
    #[error("failed to spawn video sidecar at {path}: {source}")]
    BridgeSpawn {
        /// The sidecar binary path the client attempted to execute.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Accepting the sidecar's inbound TCP connection failed.
    #[error("failed to accept video sidecar connection on {addr}: {source}")]
    BridgeAccept {
        /// The listener address the client was accepting on.
        addr: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Reading a length-delimited `BridgeEnvelope` from the sidecar
    /// connection failed.
    #[error("failed to read bridge envelope: {source}")]
    BridgeRead {
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Decoding bytes read from the sidecar connection as a
    /// `BridgeEnvelope` protobuf message failed.
    #[error("failed to decode bridge envelope: {source}")]
    BridgeDecode {
        /// Underlying `prost` decode error.
        #[source]
        source: prost::DecodeError,
    },
    /// The background bridge-reader task exited before the client was
    /// dropped, so cached telemetry or frames can no longer be updated.
    #[error("bridge reader task ended unexpectedly: {reason}")]
    ReaderTaskEnded {
        /// Human-readable description of why the task ended.
        reason: String,
    },
}
