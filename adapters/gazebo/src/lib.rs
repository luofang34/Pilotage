//! `VehicleAdapter` implementation backed by a real Gazebo diff-drive
//! vehicle, driven through a C++ gz-transport sidecar bridge over a
//! localhost TCP connection (ADR-0008). The sidecar client, framing,
//! and frame stamping live in `pilotage-sim-video`; this crate owns the
//! Gazebo-specific adapter behavior on top of them. No raw gz-transport
//! type crosses into `pilotage-protocol`.

mod adapter;
mod error;

pub use adapter::{GazeboAdapter, MOTION_SCOPE, THROTTLE_AXIS, YAW_AXIS};
pub use error::GazeboAdapterError;
