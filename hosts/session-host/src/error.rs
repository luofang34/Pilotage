//! Typed errors for the session-host binary (ADR-0015: no `anyhow` in
//! library or binary code, typed `thiserror` enums throughout).

use crate::cli::CliError;

/// Failures that can prevent the session host from starting or running.
#[derive(Debug, thiserror::Error)]
pub enum HostError {
    /// Command-line arguments were malformed.
    #[error("invalid command-line arguments: {0}")]
    Cli(#[source] CliError),
    /// Building the self-signed TLS identity for loopback development failed.
    #[error("failed to build self-signed identity: {0}")]
    Identity(#[source] wtransport::tls::error::InvalidSan),
    /// Binding or constructing the WebTransport server endpoint failed.
    #[error("failed to construct server endpoint: {0}")]
    Endpoint(#[source] std::io::Error),
    /// Reading the bound local address back from the endpoint failed.
    #[error("failed to read local address from endpoint: {0}")]
    LocalAddr(#[source] std::io::Error),
    /// Constructing the tokio runtime failed.
    #[error("failed to build the tokio runtime: {0}")]
    Runtime(#[source] std::io::Error),
    /// Spawning or connecting the Gazebo sidecar bridge failed.
    #[error("failed to start the Gazebo adapter: {0}")]
    GazeboAdapter(#[source] pilotage_adapter_gazebo::GazeboAdapterError),
}
