//! Error type for the ArduPilot adapter.

/// Why the ArduPilot adapter could not start.
#[derive(Debug, thiserror::Error)]
pub enum ArduPilotAdapterError {
    /// The MAVLink link could not start.
    #[error("starting the ArduPilot MAVLink link failed: {0}")]
    Link(#[from] pilotage_mavlink::LinkError),
}
