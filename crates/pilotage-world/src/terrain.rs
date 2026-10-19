//! The ground every simulator of one world stands on.
//!
//! A [`TerrainSource`] answers the ground height at a WGS84 position, with
//! the resolution of the data behind it and the identity of the source.
//! The simulated world takes a terrain model's heights as mean-sea-level
//! heights over the simulator's flat geoid separation
//! ([`SIMULATOR_GEOID_MODEL_ID`]): a simulator's "sea level", a vehicle's
//! GNSS altitude, and the terrain then share one vertical reference.

#[cfg(feature = "archive")]
mod archive;

#[cfg(feature = "archive")]
pub use archive::ArchiveTerrain;

use pilotage_geo::{
    BaroSettingId, GeoidModelId, LocalOriginId, SIMULATOR_GEOID_MODEL_ID, TerrainRefId,
    VerticalDatum, VerticalPosition,
};

use crate::error::TerrainError;

/// The identity of a [`UniformTerrain`]. Archive identities never take
/// it (see [`ArchiveTerrain`]).
pub const UNIFORM_TERRAIN_ID: TerrainRefId = TerrainRefId(1);

/// The ground height at one position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainSample {
    /// The height, MSL over the simulator's geoid separation.
    pub height: VerticalPosition,
    /// The ground size of one cell of the data that supplied the height,
    /// m. A uniform world has no cells and states 0.
    pub resolution_m: f64,
    /// The terrain model that supplied it.
    pub source: TerrainRefId,
}

impl TerrainSample {
    /// The height in metres above mean sea level.
    pub fn elevation_m(&self) -> f64 {
        self.height.height_m
    }
}

/// A terrain model the world samples.
pub trait TerrainSource {
    /// The identity of the model, carried by every sample.
    fn id(&self) -> TerrainRefId;

    /// The ground height at a WGS84 position, or `None` where the model
    /// has no coverage. A source may read storage, so this may block.
    ///
    /// # Errors
    ///
    /// Returns [`TerrainError`] when the model cannot be read or holds an
    /// invalid height.
    fn sample_blocking(
        &mut self,
        latitude_deg: f64,
        longitude_deg: f64,
    ) -> Result<Option<TerrainSample>, TerrainError>;
}

/// A flat world at one height everywhere: for a world with no terrain
/// model, so its ground is stated rather than missing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UniformTerrain {
    /// The height of the whole world, metres above mean sea level.
    pub elevation_m: f64,
}

impl TerrainSource for UniformTerrain {
    fn id(&self) -> TerrainRefId {
        UNIFORM_TERRAIN_ID
    }

    fn sample_blocking(
        &mut self,
        latitude_deg: f64,
        longitude_deg: f64,
    ) -> Result<Option<TerrainSample>, TerrainError> {
        Ok(Some(TerrainSample {
            height: msl_height(self.elevation_m, latitude_deg, longitude_deg)?,
            resolution_m: 0.0,
            source: UNIFORM_TERRAIN_ID,
        }))
    }
}

/// An MSL height over the simulator's geoid separation.
pub(crate) fn msl_height(
    height_m: f64,
    latitude_deg: f64,
    longitude_deg: f64,
) -> Result<VerticalPosition, TerrainError> {
    VerticalPosition::new(
        height_m,
        VerticalDatum::Msl,
        SIMULATOR_GEOID_MODEL_ID,
        TerrainRefId::UNDECLARED,
        BaroSettingId::UNDECLARED,
        LocalOriginId::UNDECLARED,
    )
    .map_err(|source| TerrainError::Height {
        height_m,
        latitude_deg,
        longitude_deg,
        source,
    })
}

/// The geoid the world's MSL heights are over.
pub const WORLD_GEOID: GeoidModelId = SIMULATOR_GEOID_MODEL_ID;

#[cfg(test)]
mod tests;
