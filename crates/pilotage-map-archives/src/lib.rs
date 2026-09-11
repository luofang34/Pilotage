//! Read installed map archives without loading complete files into memory.

mod error;
mod file_range;
mod mbtiles;
mod pmtiles;

pub use error::ArchiveError;
pub use mbtiles::MbTiles;
pub use pmtiles::PmTiles;
