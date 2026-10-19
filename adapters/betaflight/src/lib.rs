//! `VehicleAdapter` for Betaflight flight controllers over MSP.
//!
//! Betaflight speaks no MAVLink and has no velocity mode, so this adapter
//! gives a Betaflight quad the same control surface as every other
//! Pilotage multicopter by closing the loop on the host: velocity intents
//! in BODY_FRD become ANGLE-mode stick positions through an outer loop
//! that reads Betaflight's own estimate (attitude, fused altitude, GNSS).
//! A second scope in the same authority group flies it directly, the way
//! a racing pilot does: body rates in ACRO, or an attitude in ANGLE, with
//! collective thrust, each frame sent to the FC as it arrives. Arming is
//! the ARM aux switch with Betaflight's arming-disable flags as the typed
//! refusal. One task owns the MSP stream (TCP to SITL's
//! UART1, or any byte stream to a real board) and streams RC at 50 Hz;
//! when it stops, Betaflight's receiver-loss failsafe takes over.
//!
//! This crate owns I/O (`adapters/` is exempt from the sans-IO rule,
//! ADR-0002); all MSP byte work lives in `pilotage_msp`.

mod adapter;
mod config;
mod direct;
mod link;
mod local_frame;
mod outer_loop;
mod rc;
mod snapshot;

pub use adapter::{
    ARM_BUTTON, BetaflightAdapter, DIRECT_SCOPE, DISARM_BUTTON, FLIGHT_SCOPE, PITCH_AXIS,
    ROLL_AXIS, THROTTLE_AXIS, YAW_AXIS,
};
pub use config::{
    BetaflightConfig, BetaflightProfile, MAX_BODY_RATE_RPS, MAX_HORIZONTAL_MPS, MAX_TILT_RAD,
    MAX_VERTICAL_MPS, MAX_YAW_RATE_RPS,
};
