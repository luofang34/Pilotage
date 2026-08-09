//! Builds cosmetic Terrarium tiles from the terrain in a [`SourceDataset`].
//!
//! This crate reads the source dataset. It does not read or decode an SVS-02
//! package. The output is a deterministic MBTiles archive for an offline map
//! base layer. The builder converts each source height to geometric mean sea
//! level (MSL). The output has no terrain-awareness or altitude meaning.

#![forbid(unsafe_code)]

mod config;
mod error;
mod mbtiles;
mod sampler;
mod tile;

#[cfg(test)]
mod tests;

pub use config::{TerrainBuildConfig, TerrainRegion, WEB_MERCATOR_MAX_LAT_DEG};
pub use error::TerrainBuildError;
pub use pilotage_svs_build::SourceDataset;

use mbtiles::encode_archive;
use sampler::TerrainSampler;
use tile::rasterize_tiles;

/// A deterministic MBTiles archive and its build report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainBundle {
    bytes: Vec<u8>,
    report: TerrainBuildReport,
}

impl TerrainBundle {
    /// Gets the complete MBTiles archive bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Takes the complete MBTiles archive bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Gets the deterministic build report.
    #[must_use]
    pub const fn report(&self) -> &TerrainBuildReport {
        &self.report
    }
}

/// Facts that the builder gets from one archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainBuildReport {
    /// Number of PNG tiles in the archive.
    pub tile_count: u64,
    /// Size of the archive in bytes.
    pub archive_bytes: u64,
}

/// Builds one Terrarium MBTiles archive from the terrain source dataset.
///
/// # Errors
///
/// Returns [`TerrainBuildError`] if the configuration or terrain is invalid.
/// It also returns an error if the terrain does not cover a selected tile or
/// if encoding fails.
pub fn build_mbtiles(
    source: &SourceDataset,
    config: TerrainBuildConfig,
) -> Result<TerrainBundle, TerrainBuildError> {
    config.validate()?;
    let sampler = TerrainSampler::new(source)?;
    let tiles = rasterize_tiles(&sampler, config)?;
    let tile_count = tiles.len() as u64;
    let bytes = encode_archive(config, &tiles)?;
    let archive_bytes = bytes.len() as u64;
    Ok(TerrainBundle {
        bytes,
        report: TerrainBuildReport {
            tile_count,
            archive_bytes,
        },
    })
}
