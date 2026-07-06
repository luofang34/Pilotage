//! Typed errors for the `pilotage-adapter-gazebo` crate.

/// Errors this adapter's bridge connection and control/telemetry paths can
/// produce.
#[derive(Debug, thiserror::Error)]
pub enum GazeboAdapterError {
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
    /// Spawning the C++ sidecar bridge child process failed.
    #[error("failed to spawn gz-transport sidecar bridge at {path}: {source}")]
    BridgeSpawn {
        /// The bridge binary path the adapter attempted to execute.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Accepting the sidecar bridge's inbound TCP connection failed.
    #[error("failed to accept sidecar bridge connection on {addr}: {source}")]
    BridgeAccept {
        /// The listener address the adapter was accepting on.
        addr: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Connecting to the sidecar bridge's TCP endpoint failed.
    #[error("failed to connect to gz-transport sidecar at {addr}: {source}")]
    BridgeConnect {
        /// The address the adapter attempted to connect to.
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
    /// Writing a length-delimited `BridgeEnvelope` to the sidecar
    /// connection failed.
    #[error("failed to write bridge envelope: {source}")]
    BridgeWrite {
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
    /// The background bridge-reader task exited before the adapter was
    /// dropped, so cached telemetry or frames can no longer be updated.
    #[error("bridge reader task ended unexpectedly: {reason}")]
    ReaderTaskEnded {
        /// Human-readable description of why the task ended.
        reason: String,
    },
}
