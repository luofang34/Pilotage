//! Adapter projection and command admission for one MAVLink gimbal endpoint.

mod advertisement;
mod control;
mod observation;

pub use advertisement::scope_descriptor;
pub use control::apply_control;
pub use observation::{GimbalObservation, GimbalStamp, sample};

/// The payload scope has its own lease, separate from aircraft motion.
pub const SCOPE: &str = "vehicle.gimbal";
/// Logical axis for gimbal pitch.
pub const PITCH_AXIS: u16 = 1;
/// Logical axis for gimbal yaw.
pub const YAW_AXIS: u16 = 3;
/// Logical button for the native neutral-position command.
pub const NEUTRAL_BUTTON: u16 = 0;

const ENDPOINT: &str = "mavlink.gimbal.selected";

#[cfg(test)]
mod tests;
