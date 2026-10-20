//! One simulated world: a site, the terrain under it, and the magnetic
//! field over it at one epoch.

use pilotage_geo::TerrainRefId;

use world_atmosphere::{Altitude, AtmosphereModel, AtmospherePoint, AtmosphereState, Isa};

use crate::error::WorldError;
use crate::geomag::{GeomagModel, MagneticEpoch, MagneticField, WMM2025};
use crate::position::{LocalTangentPlane, local_plane, world_position};
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
    pub magnetic_model: GeomagModel,
    /// The magnetic epoch.
    pub epoch: MagneticEpoch,
}

/// One simulated world. Its air is the standard atmosphere unless
/// [`World::with_atmosphere`] gives it another model (weather).
#[derive(Debug)]
pub struct World<T, A = Isa> {
    site: Site,
    terrain: T,
    epoch: MagneticEpoch,
    air: A,
}

impl<T: TerrainSource> World<T> {
    /// A world on `site`, over `terrain`, with its field at `epoch` and
    /// the International Standard Atmosphere.
    pub fn new(site: Site, terrain: T, epoch: MagneticEpoch) -> Self {
        Self {
            site,
            terrain,
            epoch,
            air: Isa,
        }
    }
}

impl<T: TerrainSource, A: AtmosphereModel> World<T, A> {
    /// The same world with `air` as its atmosphere.
    pub fn with_atmosphere<B: AtmosphereModel>(self, air: B) -> World<T, B> {
        World {
            site: self.site,
            terrain: self.terrain,
            epoch: self.epoch,
            air,
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
        let mut placement = self.site.placement(ground, index)?;
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
    /// north-east-down components. `height_m` is a world (MSL) height; see
    /// [`crate::world_position`] for how it meets the ellipsoid.
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
        world_geomag::wmm2025(
            world_position(latitude_deg, longitude_deg, height_m),
            world_core::DecimalYear(decimal_year),
        )
        .map_err(|source| WorldError::MagneticField {
            latitude_deg,
            longitude_deg,
            height_m,
            decimal_year,
            source,
        })
    }

    /// The air at a point: the world's atmosphere model at its epoch.
    /// `height_m` is a world (MSL) height, geometric.
    ///
    /// # Errors
    ///
    /// Returns [`WorldError::Atmosphere`] outside the model's domain.
    pub fn atmosphere(
        &self,
        latitude_deg: f64,
        longitude_deg: f64,
        height_m: f64,
    ) -> Result<AtmosphereState, WorldError> {
        self.air
            .state(AtmospherePoint {
                latitude_deg,
                longitude_deg,
                altitude: Altitude::GeometricMsl(height_m),
                at: world_core::DecimalYear(self.epoch.decimal_year()),
            })
            .map_err(|source| WorldError::Atmosphere { height_m, source })
    }

    /// The local north-east-down frame tangent at a world (MSL) position.
    ///
    /// # Errors
    ///
    /// Returns [`WorldError::Position`] for a position not well formed.
    pub fn local_plane(
        &self,
        latitude_deg: f64,
        longitude_deg: f64,
        height_m: f64,
    ) -> Result<LocalTangentPlane, WorldError> {
        Ok(local_plane(latitude_deg, longitude_deg, height_m)?)
    }
}

#[cfg(test)]
mod tests;
