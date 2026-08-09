//! Fixed configuration for a cosmetic terrain archive.

use crate::TerrainBuildError;

/// North and south Web Mercator latitude limit.
pub const WEB_MERCATOR_MAX_LAT_DEG: f64 = 85.051_128_779_806_6;

/// Highest zoom that the deterministic tile index accepts.
const MAX_ZOOM: u8 = 22;

/// A WGS-84 region for the output tile selection.
///
/// The builder includes each Web Mercator tile that intersects this region.
/// The source terrain must cover each complete selected tile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainRegion {
    /// South latitude in degrees.
    pub min_lat_deg: f64,
    /// North latitude in degrees.
    pub max_lat_deg: f64,
    /// West longitude in degrees.
    pub min_lon_deg: f64,
    /// East longitude in degrees.
    pub max_lon_deg: f64,
}

impl TerrainRegion {
    fn is_finite(self) -> bool {
        self.min_lat_deg.is_finite()
            && self.max_lat_deg.is_finite()
            && self.min_lon_deg.is_finite()
            && self.max_lon_deg.is_finite()
    }

    fn is_ordered(self) -> bool {
        self.min_lat_deg < self.max_lat_deg && self.min_lon_deg < self.max_lon_deg
    }

    fn is_in_web_mercator(self) -> bool {
        self.min_lat_deg >= -WEB_MERCATOR_MAX_LAT_DEG
            && self.max_lat_deg <= WEB_MERCATOR_MAX_LAT_DEG
            && self.min_lon_deg >= -180.0
            && self.max_lon_deg <= 180.0
    }
}

/// Parameters that select the region and Web Mercator zoom range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainBuildConfig {
    /// WGS-84 bounds for the archive.
    pub region: TerrainRegion,
    /// Lowest Web Mercator zoom in the archive.
    pub min_zoom: u8,
    /// Highest Web Mercator zoom in the archive.
    pub max_zoom: u8,
}

impl TerrainBuildConfig {
    /// Checks the region and zoom range.
    ///
    /// # Errors
    ///
    /// Returns [`TerrainBuildError::InvalidConfig`] for invalid bounds or zooms.
    pub fn validate(self) -> Result<(), TerrainBuildError> {
        if !self.region.is_finite() {
            return Err(TerrainBuildError::InvalidConfig {
                reason: "region bounds must be finite",
            });
        }
        if !self.region.is_ordered() {
            return Err(TerrainBuildError::InvalidConfig {
                reason: "region bounds must be ordered",
            });
        }
        if !self.region.is_in_web_mercator() {
            return Err(TerrainBuildError::InvalidConfig {
                reason: "region must be inside the Web Mercator limits",
            });
        }
        if self.min_zoom > self.max_zoom || self.max_zoom > MAX_ZOOM {
            return Err(TerrainBuildError::InvalidConfig {
                reason: "zoom range must be ordered and no higher than 22",
            });
        }
        Ok(())
    }
}
