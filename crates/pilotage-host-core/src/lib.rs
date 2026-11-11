//! Pilotage's embeddable vehicle host.
//!
//! [`HostCore`] holds what a Pilotage host runs for one vehicle: the
//! mission principal, the session engine and the Aviate adapter. It reads
//! no clock, opens no socket and starts no task. The embedding host (a
//! simulator, a browser page, or Pilotage's own session host) gives it
//! the time on each call, moves the MAVLink bytes of the flight
//! controller's two links, and calls [`HostCore::tick`] again at the
//! time it returns. The same inputs give the same output bytes.

pub mod automation;
mod config;
pub mod connection;
mod core;
pub mod engine_actor;
mod error;
mod output;
mod registry;
pub mod wire_codec;

pub use config::HostCoreConfig;
pub use core::HostCore;
pub use error::HostCoreError;
pub use output::{HostEvent, HostStatus, NextWake};

use pilotage_protocol::VehicleId;

pub use automation::MISSION_CLOCK;

/// The vehicle the core hosts, as the session host numbers it.
pub const HOST_VEHICLE: VehicleId = VehicleId::new(1);
