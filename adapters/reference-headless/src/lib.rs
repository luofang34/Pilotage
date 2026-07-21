//! Headless reference adapter implementing `pilotage-adapter-api` without a
//! real simulation engine, used for conformance testing and local
//! development (ADR-0008).

mod adapter;
mod controls;
mod scenario;
mod skiff;
mod splitmix64;

pub use adapter::{ADAPTER_VERSION, ReferenceAdapter, ReferenceAdapterSnapshot};
pub use controls::{
    ControlState, MAX_SURGE_MPS, MAX_TURN_RPS, MOTION_SCOPE, STEERING_AXIS, THROTTLE_AXIS,
};
pub use scenario::initial_state_from_seed;
pub use skiff::{DRAG, DT_SECONDS, MAX_ACCEL, SkiffState, YAW_RATE};
pub use splitmix64::SplitMix64;
