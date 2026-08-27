use std::path::PathBuf;

use thiserror::Error;

use crate::Digest;

/// An X-Plane identity, protocol, or transport error.
#[derive(Debug, Error)]
pub enum XPlaneTrialError {
    /// An installed plugin build manifest cannot be read.
    #[error("cannot read X-Plane plugin build manifest {path:?}: {source}")]
    BuildManifestRead {
        /// The manifest path.
        path: PathBuf,
        /// The file error.
        #[source]
        source: std::io::Error,
    },
    /// An installed plugin build manifest exceeds its fixed size limit.
    #[error("X-Plane plugin build manifest {path:?} has {size} bytes")]
    BuildManifestTooLarge {
        /// The manifest path.
        path: PathBuf,
        /// The observed size.
        size: usize,
    },
    /// An installed plugin build manifest is not strict JSON.
    #[error("cannot decode X-Plane plugin build manifest {path:?}: {source}")]
    BuildManifestDecode {
        /// The manifest path.
        path: PathBuf,
        /// The JSON error.
        #[source]
        source: serde_json::Error,
    },
    /// An installed plugin build manifest has invalid content.
    #[error("invalid X-Plane plugin build manifest: {detail}")]
    InvalidBuildManifest {
        /// The validation detail.
        detail: String,
    },
    /// A listener address is not local to this computer.
    #[error("X-Plane trial listener address {address} is not loopback")]
    NonLocalAddress {
        /// The refused address.
        address: String,
    },
    /// A listener address is not valid.
    #[error("cannot bind X-Plane trial listener at {address}: {source}")]
    Bind {
        /// The requested address.
        address: String,
        /// The socket error.
        #[source]
        source: std::io::Error,
    },
    /// A listener operation failed.
    #[error("X-Plane trial listener failed during {operation}: {source}")]
    Listener {
        /// The operation name.
        operation: &'static str,
        /// The socket error.
        #[source]
        source: std::io::Error,
    },
    /// A session transport operation failed.
    #[error("X-Plane trial session failed during {operation}: {source}")]
    SessionIo {
        /// The operation name.
        operation: &'static str,
        /// The socket error.
        #[source]
        source: std::io::Error,
    },
    /// A sample read failed and the prior socket timeout could not be restored.
    #[error(
        "X-Plane sample read failed ({read_error}); cannot restore the socket timeout: {source}"
    )]
    ReadTimeoutRestore {
        /// The original sample-read error.
        read_error: String,
        /// The timeout restoration error.
        #[source]
        source: std::io::Error,
    },
    /// The peer closed before it sent a complete line.
    #[error("X-Plane trial peer closed during {operation}")]
    PeerClosed {
        /// The active operation.
        operation: &'static str,
    },
    /// A protocol line is not valid.
    #[error("invalid X-Plane trial protocol line: {detail}")]
    InvalidProtocol {
        /// The parse detail.
        detail: String,
    },
    /// The plugin uses another protocol version.
    #[error("X-Plane trial protocol {actual} does not match required {expected}")]
    ProtocolVersion {
        /// The required version.
        expected: u32,
        /// The plugin version.
        actual: u32,
    },
    /// A reported path is not the expected path.
    #[error("X-Plane {artifact} path {actual:?} does not match {expected:?}")]
    ArtifactPath {
        /// The artifact name.
        artifact: &'static str,
        /// The expected path.
        expected: PathBuf,
        /// The reported path.
        actual: PathBuf,
    },
    /// A file cannot be opened for identity calculation.
    #[error("cannot read X-Plane {artifact} file {path:?}: {source}")]
    ArtifactRead {
        /// The artifact name.
        artifact: &'static str,
        /// The file path.
        path: PathBuf,
        /// The file error.
        #[source]
        source: std::io::Error,
    },
    /// A file identity does not match the expected value.
    #[error("X-Plane {artifact} digest {actual} does not match {expected}")]
    ArtifactDigest {
        /// The artifact name.
        artifact: &'static str,
        /// The expected digest.
        expected: Digest,
        /// The actual digest.
        actual: Digest,
    },
    /// The configured model identity is empty.
    #[error("X-Plane simulator model digest is zero")]
    ZeroModelDigest,
    /// The loaded trial plugin does not contain the expected source build.
    #[error("loaded X-Plane trial source build {actual:?} does not match {expected:?}")]
    TrialSourceBuild {
        /// The required source build identity.
        expected: String,
        /// The identity reported by the loaded plugin.
        actual: String,
    },
    /// The trial plugin was not built with the expected bridge binary.
    #[error("loaded X-Plane bridge bundle digest {actual} does not match {expected}")]
    LoadedBridgeDigest {
        /// The required bridge binary digest.
        expected: Digest,
        /// The bridge digest embedded in the loaded trial plugin.
        actual: Digest,
    },
    /// A command receipt does not match the request.
    #[error("X-Plane trial receipt does not match: {detail}")]
    ReceiptMismatch {
        /// The mismatch detail.
        detail: String,
    },
    /// The plugin rejected a command.
    #[error("X-Plane trial command {generation} was rejected: {code}")]
    CommandRejected {
        /// The command generation.
        generation: u64,
        /// The stable rejection code.
        code: String,
    },
    /// A command is not valid in the session state.
    #[error("X-Plane trial state does not permit {operation}")]
    InvalidState {
        /// The operation name.
        operation: &'static str,
    },
}
