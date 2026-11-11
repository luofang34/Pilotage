//! Pilotage's embeddable vehicle host.
//!
//! [`HostCore`] holds what a Pilotage host runs for one vehicle: the
//! mission principal, the session engine and the Aviate adapter. It reads
//! no clock, opens no socket and starts no task. The embedding host (a
//! simulator, a browser page, or Pilotage's own session host) gives it
//! the time on each call, moves the MAVLink bytes of the flight
//! controller's two links, and calls [`HostCore::tick`] again at the
//! time it returns. The same inputs give the same output bytes.

mod config;
mod core;
mod error;
mod output;

pub use config::HostCoreConfig;
pub use core::HostCore;
pub use error::HostCoreError;
pub use output::{HostEvent, HostStatus, NextWake};
