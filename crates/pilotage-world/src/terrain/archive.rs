//! Terrain from a Terrarium MBTiles archive.

use std::path::Path;

use pilotage_geo::TerrainRefId;
use pilotage_terrain_query::{TerrainArchive, TerrainQueryError};

use super::{TerrainSample, TerrainSource, msl_height};
use crate::error::TerrainError;

/// WGS84 equatorial circumference, m: the Web Mercator tile scale.
const EQUATOR_M: f64 = 2.0 * core::f64::consts::PI * 6_378_137.0;

/// A Terrarium archive as the world's terrain.
pub struct ArchiveTerrain {
    archive: TerrainArchive,
    id: TerrainRefId,
}

impl ArchiveTerrain {
    /// Opens the archive at `path`. Its identity is a hash of its `name`
    /// and `version` metadata, so two copies of one archive are one
    /// terrain model and a new version is another.
    ///
    /// # Errors
    ///
    /// Returns [`TerrainQueryError`] when the archive cannot be opened
    /// or does not hold Terrarium tiles.
    pub fn open_blocking(path: impl AsRef<Path>) -> Result<Self, TerrainQueryError> {
        let archive = TerrainArchive::open_blocking(path)?;
        let (name, version) = archive.identity_blocking()?;
        Ok(Self {
            archive,
            id: archive_id(&name, version.as_deref()),
        })
    }
}

/// FNV-1a over `name`, a separator, and `version` (a second separator
/// when there is none), with the top bit set so it never equals
/// [`super::UNIFORM_TERRAIN_ID`] or the undeclared id.
pub(crate) fn archive_id(name: &str, version: Option<&str>) -> TerrainRefId {
    let version = match version {
        Some(version) => version.as_bytes(),
        None => &[0],
    };
    let mut hash: u32 = 0x811c_9dc5;
    for byte in name.bytes().chain([0]).chain(version.iter().copied()) {
        hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
    }
    TerrainRefId(hash | 0x8000_0000)
}

/// The ground size of one pixel of a `tile_size` tile at `zoom`, at
/// `latitude_deg`, m.
pub(crate) fn pixel_size_m(latitude_deg: f64, zoom: u8, tile_size: u32) -> f64 {
    let pixels = f64::from(tile_size) * libm::pow(2.0, f64::from(zoom));
    EQUATOR_M * libm::cos(latitude_deg.to_radians()) / pixels
}

impl TerrainSource for ArchiveTerrain {
    fn id(&self) -> TerrainRefId {
        self.id
    }

    fn sample_blocking(
        &mut self,
        latitude_deg: f64,
        longitude_deg: f64,
    ) -> Result<Option<TerrainSample>, TerrainError> {
        let Some(sample) = self
            .archive
            .elevation_sample_blocking(latitude_deg, longitude_deg)?
        else {
            return Ok(None);
        };
        Ok(Some(TerrainSample {
            height: msl_height(sample.elevation_m, latitude_deg, longitude_deg)?,
            resolution_m: pixel_size_m(latitude_deg, sample.zoom, sample.tile_size),
            source: self.id,
        }))
    }
}
