//! Sans-IO feeder: stamped wire telemetry in, instrument state out.
//!
//! Every shell links this one implementation (ADR-0029: client script
//! holds no wire- or measurement-interpreting logic): the browser drives
//! it as wasm behind thin script wrappers, a native shell links it
//! through FFI, and a host posture links it directly. Inputs are plain
//! per-source group samples with identity and acquisition stamps —
//! no protocol types, no transport; time enters only as the caller's
//! clocks.
//!
//! The pieces mirror the admission discipline they enforce:
//! [`avionics::AvionicsIngress`] gates the operational-estimate lanes
//! (AV-01 identity, wrap-safe ordering, coherence, and the fail-closed
//! authorization regimes), [`fc_state::FcStateTracker`] and
//! [`nav_guidance::NavGuidanceTracker`] pin their single-source lanes,
//! [`turn::TurnDerivation`] differences heading only within one stream
//! (DYN-01), and [`nav_display`] is the one place wire meters become
//! instrument dots (ADR-0031).

#![no_std]

#[cfg(test)]
extern crate std;

pub mod avionics;
pub mod fc_state;
pub mod nav_display;
pub mod nav_guidance;
pub mod stamp;
pub mod turn;

pub use stamp::{
    CLOCK_HOST_MONOTONIC, CLOCK_SIMULATION, CLOCK_VEHICLE_BOOT, ROLE_FC_STATE,
    ROLE_NAVIGATION_SOLUTION, ROLE_OPERATIONAL_ESTIMATE, ROLE_SIMULATION_TRUTH, RawStamp,
    StampFault, serial_is_newer, stamp_fault_for_role,
};
