//! `VehicleAdapter` for ArduCopter over MAVLink 2.0 (ADR-0018).
//!
//! Telemetry rides the shared [`pilotage_mavlink`] link with ArduPilot's
//! EKF_STATUS_REPORT as the authorization source, sampled by the same
//! [`pilotage_mavlink::sampling`] code every MAVLink adapter uses. Control
//! is ArduCopter's GUIDED mode: the canonical body-frame velocity intent
//! maps directly onto a body-frame velocity and yaw-rate setpoint, a climb
//! intent while landed becomes a takeoff, and ArduCopter's own guided
//! timeout is the FC-side twin of the host's link-loss policy. A second
//! scope in the same authority group flies direct: body rates or attitude
//! with collective thrust, as GUIDED SET_ATTITUDE_TARGET.
//!
//! This crate owns I/O (`adapters/` is exempt from the sans-IO rule,
//! ADR-0002); all MAVLink byte work lives in `pilotage_mavlink`.

mod adapter;
mod config;
mod error;
mod uplink;

pub use adapter::{
    ARM_BUTTON, ArduPilotAdapter, DIRECT_SCOPE, DISARM_BUTTON, FLIGHT_SCOPE, PITCH_AXIS, ROLL_AXIS,
    THROTTLE_AXIS, YAW_AXIS,
};
pub use config::{ArduPilotConfig, ArduPilotProfile};
pub use error::ArduPilotAdapterError;
pub use uplink::{
    MAX_BODY_RATE_RPS, MAX_DIRECT_TILT_RAD, MAX_HORIZONTAL_MPS, MAX_VERTICAL_MPS, MAX_YAW_RATE_RPS,
    TAKEOFF_ALTITUDE_M,
};
