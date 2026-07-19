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
    /// `PILOTAGE_AVIATE_PROFILE` held a value that is not a known session
    /// profile. Unknown values fail startup rather than falling back: a
    /// typo in a physical deployment must never fail open into the
    /// simulation profile.
    #[error(
        "invalid PILOTAGE_AVIATE_PROFILE value {value:?} (expected physical, simulation, or oracle-only)"
    )]
    AviateProfile {
        /// The rejected value, lossily decoded for this message.
        value: String,
    },
    /// Spawning or connecting the Gazebo sidecar bridge failed.
    #[error("failed to start the Gazebo adapter: {0}")]
    GazeboAdapter(#[source] pilotage_adapter_gazebo::GazeboAdapterError),
    /// Starting the Aviate MAVLink telemetry link failed.
    #[error("failed to start the Aviate adapter: {0}")]
    AviateAdapter(#[source] pilotage_adapter_aviate::AviateAdapterError),
    /// Starting the PX4 MAVLink link failed.
    #[error("failed to start the PX4 adapter: {0}")]
    Px4Adapter(#[source] pilotage_adapter_px4::Px4AdapterError),
}
