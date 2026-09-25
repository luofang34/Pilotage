//! The Aircraft domain: profile, loading, and derived performance (ADR-0046).
//!
//! An [`AircraftProfile`] is versioned data with a content hash
//! ([`ProfileId`]). Its [`EnergyStore`] is fuel or a battery, and the
//! calculators refuse to mix the two. A [`Loading`] records one flight's
//! station weights and fuel. The calculators return results that name the
//! profile hash and the inputs they used, with a validity time, so a consumer
//! can see when a result is stale. The calculators are advisory.
//!
//! SIM / NOT FOR FLIGHT.

#![forbid(unsafe_code)]

mod balance;
mod endurance;
mod envelope;
mod error;
mod loading;
mod profile;

pub use balance::{WeightAndBalance, weight_and_balance};
pub use endurance::{Endurance, EnergyState, Remaining, endurance};
pub use error::AircraftError;
pub use loading::Loading;
pub use profile::{AircraftProfile, CruiseModel, Draw, EnergyStore, ProfileId, Station, Tank};
