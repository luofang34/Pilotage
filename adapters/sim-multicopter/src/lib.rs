//! A simulated multicopter on the DJI Mini 3 control path.
//!
//! This adapter is the test counterpart of `pilotage-adapter-dji`. It
//! advertises the same flight and gimbal scopes, the same velocity and
//! gimbal-rate intents, and the same Takeoff and Land actions, and it
//! publishes the same telemetry groups. A mission flies it as it flies the
//! DJI aircraft. The differences are explicit and machine-readable:
//! `ExecutionMode::physically_embodied` is false, the display name starts
//! with `SIM`, and a simulation-truth group is present.
//!
//! The gimbal camera renders frames from web-mercator aerial imagery on
//! a worker thread, with optional injected faults.

mod adapter;
mod battery;
pub mod camera;
mod config;
mod geodesy;
mod gimbal;
mod kinematics;
mod lifecycle;
pub mod tiles;

pub use adapter::{GIMBAL_SCOPE, MOTION_SCOPE, SimMulticopterAdapter, TICK_NS};
pub use config::{
    BatteryConfig, CameraConfig, ESRI_WORLD_IMAGERY, Limits, SimConfig, TileSourceConfig,
};
pub use geodesy::{LaunchPoint, LocalFrame, quaternion_from_euler};
pub use kinematics::Kinematics;
pub use lifecycle::{FlightPhase, TAKEOFF_HEIGHT_M};
