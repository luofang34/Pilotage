//! Why the world could not answer.

use pilotage_geo::GeoError;

/// Why a terrain source could not answer.
#[derive(Debug, thiserror::Error)]
pub enum TerrainError {
    /// The archive could not be read.
    #[cfg(feature = "archive")]
    #[error("terrain archive query failed: {0}")]
    Archive(#[from] pilotage_terrain_query::TerrainQueryError),
    /// The source produced a height that is not a valid MSL height.
    #[error("terrain height {height_m} m at {latitude_deg}, {longitude_deg} is invalid: {source}")]
    Height {
        /// The rejected height.
        height_m: f64,
        /// Where it was sampled, degrees north.
        latitude_deg: f64,
        /// Where it was sampled, degrees east.
        longitude_deg: f64,
        /// Why it was rejected.
        #[source]
        source: GeoError,
    },
}

/// Why the world could not answer.
#[derive(Debug, thiserror::Error)]
pub enum WorldError {
    /// The terrain source failed.
    #[error("terrain: {0}")]
    Terrain(#[from] TerrainError),
    /// The magnetic model refused the point or the epoch.
    #[error(
        "magnetic field at {latitude_deg}, {longitude_deg}, {height_m} m in {decimal_year}: {source}"
    )]
    MagneticField {
        /// Where the field was asked for, degrees north.
        latitude_deg: f64,
        /// Where the field was asked for, degrees east.
        longitude_deg: f64,
        /// Height above the ellipsoid, m.
        height_m: f64,
        /// The world's epoch, decimal year.
        decimal_year: f64,
        /// The model's refusal.
        #[source]
        source: world_geomag::GeomagError,
    },
}
