//! The Aircraft domain: profile, loading, and derived performance (ADR-0046).
//!
//! An [`AircraftProfile`] is versioned data with a content hash
//! ([`ProfileId`]). A [`Loading`] records one flight's station weights and
//! fuel. The calculators return results that name the profile hash and the
//! inputs they used, with a validity time, so a consumer can see when a
//! result is stale. The calculators are advisory.
//!
//! SIM / NOT FOR FLIGHT.

#![forbid(unsafe_code)]

mod balance;
mod endurance;
mod error;
mod profile;

pub use balance::{WeightAndBalance, weight_and_balance};
pub use endurance::{Endurance, FuelState, endurance};
pub use error::AircraftError;
pub use profile::{AircraftProfile, CruiseModel, Loading, ProfileId, Station, Tank};
