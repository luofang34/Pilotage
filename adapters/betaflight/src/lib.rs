//! `VehicleAdapter` for Betaflight flight controllers over MSP.
//!
//! Betaflight speaks no MAVLink and has no velocity mode, so this adapter
//! gives a Betaflight quad the same control surface as every other
//! Pilotage multicopter by closing the loop on the host: velocity intents
//! in BODY_FRD become ANGLE-mode stick positions through an outer loop
//! that reads Betaflight's own estimate (attitude, fused altitude, GNSS),
//! and arming is the ARM aux switch with Betaflight's arming-disable flags
//! as the typed refusal. One task owns the MSP stream (TCP to SITL's
//! UART1, or any byte stream to a real board) and streams RC at 50 Hz;
//! when it stops, Betaflight's receiver-loss failsafe takes over.
//!
//! This crate owns I/O (`adapters/` is exempt from the sans-IO rule,
//! ADR-0002); all MSP byte work lives in `pilotage_msp`.

mod adapter;
mod config;
mod link;
mod local_frame;
mod outer_loop;
mod rc;
mod snapshot;

pub use adapter::{
    ARM_BUTTON, BetaflightAdapter, DISARM_BUTTON, FLIGHT_SCOPE, PITCH_AXIS, ROLL_AXIS,
    THROTTLE_AXIS, YAW_AXIS,
};
pub use config::{
    BetaflightConfig, BetaflightProfile, MAX_HORIZONTAL_MPS, MAX_VERTICAL_MPS, MAX_YAW_RATE_RPS,
};
