//! `VehicleAdapter` for the PX4 autopilot over standard MAVLink 2.0
//! (ADR-0018).
//!
//! PX4 exposes the same wire face regardless of which simulator (or
//! airframe) sits behind it, so this adapter is FDM-agnostic: Gazebo,
//! JSBSim, or FlightGear all look identical from here. Telemetry rides
//! the shared [`pilotage_mavlink`] receive link (attitude, local
//! position, and the standard ESTIMATOR_STATUS as the authorization
//! source); control is the offboard contract — a continuous velocity
//! setpoint stream that must precede and accompany OFFBOARD mode, with
//! PX4's own offboard-loss failsafe as the FC-side twin of the host's
//! link-loss policy.
//!
//! This crate owns I/O (`adapters/` is exempt from the sans-IO rule,
//! ADR-0002); all MAVLink byte work lives in `pilotage_mavlink`.

mod adapter;
mod error;
mod uplink;

pub use adapter::{
    ARM_BUTTON, DISARM_BUTTON, FLIGHT_SCOPE, PITCH_AXIS, Px4Adapter, RESET_BUTTON, ROLL_AXIS,
    THROTTLE_AXIS, YAW_AXIS,
};
pub use error::Px4AdapterError;
pub use uplink::Px4Uplink;
