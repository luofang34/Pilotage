//! Reads terrain elevation from a Terrarium MBTiles archive.
//!
//! The reader selects the deepest tile that covers a WGS84 position. It uses
//! a less detailed tile when a sparse archive has no tile at the first zoom.

#![forbid(unsafe_code)]

mod archive;
mod error;
mod tile;

#[cfg(test)]
mod tests;

pub use archive::TerrainArchive;
pub use error::TerrainQueryError;
