//! Unified aircraft/navigation state and per-signal validity (ADR-0017).
//!
//! Every instrument — PFD, HSI, six-pack, engine page — is a display
//! surface over the one state model defined here, never over its own
//! private data. The model has two halves:
//!
//! - **Input state** ([`AircraftState`]): raw estimate groups (attitude,
//!   kinematics, air data, nav) each stamped with its age, plus source
//!   quality/validity, exactly as a feeder (telemetry bridge, local
//!   sensors, test harness) wrote them.
//! - **Resolved state** ([`PanelData`]): display-ready quantities, each a
//!   [`Sig`] carrying a [`SignalStatus`] resolved from freshness and
//!   source validity. Panels render status honestly — dashes for
//!   `Missing`, flags for `Stale`, red-X for `Failed` — and never hold
//!   last-good values silently.
//!
//! The crate is `no_std`, allocation-free, and sans-IO: time enters only
//! as ages the caller supplies. [`abi`] defines the packed little-endian
//! input layout shared with non-Rust feeders (the browser writes it into
//! WASM linear memory). Decoding and resolution are fail-safe (VAL-01):
//! trust must be declared, unknown wire values fail rather than mapping
//! to benign ones, and no non-finite value can reach scene generation.

#![no_std]

pub mod abi;
mod aircraft;
mod altitude;
mod presentation;
mod resolve;
mod signal;
pub mod units;
mod validate;

pub use aircraft::{
    AirData, AircraftState, Attitude, EstimateQuality, Kinematics, NavData, NavFromTo, NavSource,
    Selections, SnapshotCoherence, SnapshotMeta, Stamped, ValidFlags, Wind,
};
pub use altitude::{AltitudeClass, AltitudeDeclaration, AltitudeReference, GeoidModelId, OriginId};
pub use pilotage_frames::Quat;
pub use presentation::{
    AirframeDisplayProfile, AttitudePresentation, ChevronSense, Hysteresis, ProfileError,
    ProfileLimits, UnusualAttitudeState, down_in_body,
};
pub use resolve::{
    BARO_SETTING_TOLERANCE_HPA, NavResolved, PanelData, ResolvedAltitude, resolve, resolve_stateful,
};
pub use signal::{FreshnessPolicy, PolicyError, Sig, SignalStatus};
pub use validate::{
    GroupFault, QUAT_NORM_TOLERANCE, StateIntegrity, validate_quat, validate_state,
};
