//! Typed errors for the `pilotage-adapter-gazebo` crate.

use pilotage_sim_video::SimVideoError;

/// Errors this adapter's bridge connection and control/telemetry paths can
/// produce.
#[derive(Debug, thiserror::Error)]
pub enum GazeboAdapterError {
    /// The sidecar video link failed (spawn, connect, framing, or its
    /// background reader ending).
    #[error(transparent)]
    Video(#[from] SimVideoError),
    /// Drawing an opaque attachment incarnation for the camera capture
    /// identity from the operating-system CSPRNG failed.
    #[error("failed to obtain a capture incarnation from the OS CSPRNG: {source}")]
    IncarnationUnavailable {
        /// Underlying `getrandom` error.
        #[source]
        source: getrandom::Error,
    },
}
