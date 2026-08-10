//! Builds deterministic offline vector tiles from one Navdata snapshot.
//!
//! The builder accepts an immutable snapshot with its cycle, snapshot identity,
//! and digest. It emits gzip-compressed Mapbox Vector Tiles in one MBTiles 1.3
//! archive. It does not read a provider source or use a network.

#![forbid(unsafe_code)]

mod archive;
mod build;
mod config;
mod error;
mod feature;
mod geometry;
mod mercator;
mod model;
mod mvt;
mod reader;
mod source;
mod tile;

#[cfg(test)]
mod tests;

pub use build::build_mbtiles;
pub use config::NavdataTileConfig;
pub use error::NavdataTileError;
pub use model::{
    BaselineFeatureCounts, NavdataTileBundle, NavdataTileReport, OmittedFeatureCounts,
};
pub use reader::OfflineTileReader;

/// Schema version for the Navdata baseline tile bundle.
pub const NAVDATA_TILE_SCHEMA_VERSION: u16 = 1;
