//! `VehicleAdapter` implementation backed by a real Gazebo diff-drive
//! vehicle, driven through a C++ gz-transport sidecar bridge over a
//! localhost TCP connection (ADR-0008). This crate owns all I/O
//! (`adapters/` is exempt from the sans-IO rule, ADR-0002); no raw
//! gz-transport type crosses into `pilotage-protocol`.

mod adapter;
mod bridge_client;
mod error;
mod framing;
mod video;
pub mod wire;

pub use adapter::{
    CHASE_CAMERA, CHASE_SOURCE_ID, FPV_CAMERA, FPV_SOURCE_ID, GIMBAL_CAMERA, GIMBAL_SOURCE_ID,
    GazeboAdapter, MOTION_SCOPE, RawVideoFrame, THROTTLE_AXIS, YAW_AXIS,
};
pub use bridge_client::{BRIDGE_BIN_ENV, BridgeClient, BridgeConfig, LatestBridgeState};
pub use error::GazeboAdapterError;
pub use video::FrameStamper;
