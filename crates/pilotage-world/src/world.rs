//! One simulated world: a site, the terrain under it, and the magnetic
//! field over it at one epoch.

use pilotage_geo::TerrainRefId;

use crate::error::WorldError;
use crate::geomag::{MagneticEpoch, MagneticField, ModelId, WMM2025};
use crate::site::{Ground, Placement, Site};
use crate::terrain::{TerrainSample, TerrainSource};

/// Where a site's ground height came from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ElevationOrigin {
    /// The world's terrain covers the point and supplied the height.
    Terrain(TerrainSample),
    /// The terrain has no coverage there; the site's declared height
    /// stands.
    Declared,
}

/// A ground height and where it came from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SiteElevation {
    /// Metres above mean sea level.
    pub elevation_m: f64,
    /// Where the height came from.
    pub origin: ElevationOrigin,
}

/// Which models a world answers from, for provenance on what a simulator
/// derives from it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldSources {
    /// The terrain model.
    pub terrain: TerrainRefId,
    /// The magnetic model.
    pub magnetic_model: ModelId,
    /// The magnetic epoch.
    pub epoch: MagneticEpoch,
}

/// One simulated world.
#[derive(Debug)]
pub struct World<T> {
    site: Site,
    terrain: T,
    epoch: MagneticEpoch,
}

impl<T: TerrainSource> World<T> {
    /// A world on `site`, over `terrain`, with its field at `epoch`.
    pub fn new(site: Site, terrain: T, epoch: MagneticEpoch) -> Self {
        Self {
            site,
            terrain,
            epoch,
        }
    }

    /// The site.
    pub fn site(&self) -> Site {
        self.site
    }

    /// The models the world answers from.
    pub fn sources(&self) -> WorldSources {
        WorldSources {
            terrain: self.terrain.id(),
            magnetic_model: WMM2025,
            epoch: self.epoch,
        }
    }

    /// The ground height at a WGS84 position, or `None` outside the
    /// terrain's coverage.
    ///
    /// # Errors
    ///
    /// Returns [`WorldError::Terrain`] when the terrain cannot be read.
    pub fn elevation_blocking(
        &mut self,
        latitude_deg: f64,
        longitude_deg: f64,
    ) -> Result<Option<TerrainSample>, WorldError> {
        Ok(self.terrain.sample_blocking(latitude_deg, longitude_deg)?)
    }

    /// The ground height at the site's threshold: the terrain's where it
    /// covers the threshold, else the site's declared height.
    ///
    /// # Errors
    ///
    /// Returns [`WorldError::Terrain`] when the terrain cannot be read.
    pub fn site_elevation_blocking(&mut self) -> Result<SiteElevation, WorldError> {
        self.ground_at_blocking(
            self.site.latitude_deg,
            self.site.longitude_deg,
            self.site.elevation_m,
        )
    }

    /// Where the `index`-th vehicle on `ground` starts, standing on the
    /// terrain at its own point where the terrain covers it.
    ///
    /// # Errors
    ///
    /// Returns [`WorldError::Terrain`] when the terrain cannot be read.
    pub fn placement_blocking(
        &mut self,
        ground: Ground,
        index: usize,
    ) -> Result<Placement, WorldError> {
        let mut placement = self.site.placement(ground, index);
        placement.elevation_m = self
            .ground_at_blocking(
                placement.latitude_deg,
                placement.longitude_deg,
                placement.elevation_m,
            )?
            .elevation_m;
        Ok(placement)
    }

    fn ground_at_blocking(
        &mut self,
        latitude_deg: f64,
        longitude_deg: f64,
        declared_m: f64,
    ) -> Result<SiteElevation, WorldError> {
        Ok(
            match self.elevation_blocking(latitude_deg, longitude_deg)? {
                Some(sample) => SiteElevation {
                    elevation_m: sample.elevation_m(),
                    origin: ElevationOrigin::Terrain(sample),
                },
                None => SiteElevation {
                    elevation_m: declared_m,
                    origin: ElevationOrigin::Declared,
                },
            },
        )
    }

    /// The main magnetic field at a point, at the world's epoch, in
    /// north-east-down components.
    ///
    /// `height_m` is above the WGS84 ellipsoid. The world's MSL heights
    /// are over a flat simulator geoid, so a caller passes them here
    /// unchanged; the tens of metres a real geoid would add move the
    /// field by far less than the model's own uncertainty.
    ///
    /// # Errors
    ///
    /// Returns [`WorldError::MagneticField`] for a point outside the
    /// model's domain or an epoch outside its validity.
    pub fn magnetic_field(
        &self,
        latitude_deg: f64,
        longitude_deg: f64,
        height_m: f64,
    ) -> Result<MagneticField, WorldError> {
        let decimal_year = self.epoch.decimal_year();
        aviate_geomag::wmm2025(
            aviate_geomag::GeodeticPoint {
                latitude_deg,
                longitude_deg,
                height_m,
            },
            decimal_year,
        )
        .map_err(|source| WorldError::MagneticField {
            latitude_deg,
            longitude_deg,
            height_m,
            decimal_year,
            source,
        })
    }
}

#[cfg(test)]
mod tests;
