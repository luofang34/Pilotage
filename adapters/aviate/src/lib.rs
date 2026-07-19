//! `VehicleAdapter` for the Aviate flight controller (ADR-0018).
//!
//! Aviate's public contract is a deliberate MAVLink 2.0 subset covering
//! liveness, estimator values, lossless estimator authorization, and flight
//! commands. The FC exposes it over UDP in SITL and USB CDC on hardware.
//! This adapter folds the estimate into the telemetry plane: planar pose for
//! existing consumers and raw groups plus explicit validity/quality into
//! `AvionicsSample` for the instrument runtime (ADR-0017). Aviate's private
//! estimator-status message is the sole authorization source; the standard
//! status projection remains diagnostic.
//!
//! This crate owns I/O (`adapters/` is exempt from the sans-IO rule,
//! ADR-0002); the MAVLink frame math itself lives in [`mavlink`] as pure
//! byte functions so it is unit-testable byte-for-byte.

mod adapter;
mod error;
mod incarnation;
pub mod shm;
mod uplink;

pub use pilotage_mavlink::codec as mavlink;

pub use adapter::{
    ARM_BUTTON, AviateAdapter, AviateProfile, DISARM_BUTTON, FLIGHT_SCOPE, PITCH_AXIS, ROLL_AXIS,
    THROTTLE_AXIS, YAW_AXIS,
};
pub use error::AviateAdapterError;
pub use incarnation::{IncarnationProvider, OsIncarnationProvider};
pub use pilotage_mavlink::link::{LinkConfig, ResetPolicy};
pub use uplink::FlightUplink;
