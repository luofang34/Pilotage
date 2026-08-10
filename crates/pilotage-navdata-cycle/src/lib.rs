//! Load one published navigation-data cycle and answer the questions a client asks of it.
//!
//! The situation client already consumes a `NavDataSnapshot`: the airspace resolver reads
//! its airspaces, the tile builder reads its points, and the weather layer needs a
//! position for each reporting station. Nothing produced a snapshot from a published
//! cycle, so each of those ran on a fixture.
//!
//! This crate is the seam. It reads an encoded cycle and hands back the snapshot the rest
//! of the pipeline already understands. It fetches nothing and it decodes no source
//! format: `aerocontext-navdata` owns both, and this crate owns only the boundary.

mod load;
mod station;

#[cfg(test)]
mod tests;

pub use load::{CycleLoadError, load_cycle, load_cycle_bytes};
pub use station::{StationPosition, weather_station_positions};
