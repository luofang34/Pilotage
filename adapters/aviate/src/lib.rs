//! Telemetry-only `VehicleAdapter` for the Aviate flight controller
//! (ADR-0018).
//!
//! Aviate's public contract is a deliberate MAVLink 2.0 subset —
//! HEARTBEAT, ATTITUDE_QUATERNION, LOCAL_POSITION_NED — over UDP in SITL
//! and USB CDC on hardware. This adapter consumes that subset and folds
//! it into the telemetry plane: planar pose for existing consumers, the
//! raw estimate into `AvionicsSample` for the instrument runtime
//! (ADR-0017). Control is not implemented in this increment: the adapter
//! advertises no controllable scopes and rejects every control frame at
//! the boundary.
//!
//! This crate owns I/O (`adapters/` is exempt from the sans-IO rule,
//! ADR-0002); the MAVLink frame math itself lives in [`mavlink`] as pure
//! byte functions so it is unit-testable byte-for-byte.

mod adapter;
mod error;
mod link;
pub mod mavlink;
pub mod shm;

pub use adapter::{AviateAdapter, AviateLinkMode};
pub use error::AviateAdapterError;
pub use link::LinkConfig;
