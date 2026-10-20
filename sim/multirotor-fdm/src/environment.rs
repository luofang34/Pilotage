//! Where the quad flies: the local frame, gravity, and air at its origin,
//! all from the shared world model.
//!
//! Heights are above mean sea level, as the world states them; the
//! simulated world reads them as ellipsoidal heights
//! (`pilotage_world::world_position` states that assumption).

use pilotage_world::{LocalTangentPlane, Ned, PositionError, local_plane, world_position};
use world_atmosphere::{Altitude, AtmosphereError, Isa};
use world_core::{GeodeticPosition, normal_gravity};

use crate::rotation::Vec3;

/// The geodetic point the local NED frame is tangent to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Origin {
    /// Latitude, degrees.
    pub lat_deg: f64,
    /// Longitude, degrees.
    pub lon_deg: f64,
    /// Altitude above mean sea level, m.
    pub alt_m: f64,
}

/// Why the environment cannot answer.
#[derive(Debug, thiserror::Error)]
pub enum EnvironmentError {
    /// The origin is not a well-formed position.
    #[error("origin {lat_deg}°, {lon_deg}° at {alt_m} m is not a usable position: {source}")]
    Origin {
        /// Origin latitude, degrees.
        lat_deg: f64,
        /// Origin longitude, degrees.
        lon_deg: f64,
        /// Origin altitude, m MSL.
        alt_m: f64,
        /// What the world model refused.
        #[source]
        source: PositionError,
    },
    /// The standard atmosphere has no air at this altitude.
    #[error("no standard atmosphere at {alt_m} m MSL: {source}")]
    Atmosphere {
        /// The altitude asked for, m MSL.
        alt_m: f64,
        /// What the atmosphere model refused.
        #[source]
        source: AtmosphereError,
    },
}

/// The local frame at the origin and the gravity the quad falls with.
#[derive(Debug, Clone, Copy)]
pub struct Environment {
    frame: LocalTangentPlane,
    gravity_mps2: f64,
}

impl Environment {
    /// The environment around `origin`. Gravity is the WGS-84 normal
    /// gravity at the origin; a quad stays close enough to it that the
    /// change with height and latitude is far below its sensor noise.
    ///
    /// # Errors
    ///
    /// [`EnvironmentError::Origin`] for an origin that is not a
    /// well-formed position.
    pub fn new(origin: Origin) -> Result<Self, EnvironmentError> {
        let refused = |source| EnvironmentError::Origin {
            lat_deg: origin.lat_deg,
            lon_deg: origin.lon_deg,
            alt_m: origin.alt_m,
            source,
        };
        let frame = local_plane(origin.lat_deg, origin.lon_deg, origin.alt_m).map_err(refused)?;
        let gravity_mps2 =
            normal_gravity(world_position(origin.lat_deg, origin.lon_deg, origin.alt_m))
                .map_err(refused)?;
        Ok(Self {
            frame,
            gravity_mps2,
        })
    }

    /// Gravity at the origin, m/s².
    #[must_use]
    pub fn gravity_mps2(&self) -> f64 {
        self.gravity_mps2
    }

    /// The local NED frame.
    #[must_use]
    pub fn frame(&self) -> &LocalTangentPlane {
        &self.frame
    }

    /// The geodetic position of a local NED `position`.
    #[must_use]
    pub fn geodetic(&self, position: Vec3) -> GeodeticPosition {
        self.frame.to_geodetic(Ned {
            north_m: position.x,
            east_m: position.y,
            down_m: position.z,
        })
    }

    /// Static pressure of the standard atmosphere at `position`, Pa.
    ///
    /// # Errors
    ///
    /// [`EnvironmentError::Atmosphere`] where the model has no air.
    pub fn pressure_pa(&self, position: GeodeticPosition) -> Result<f64, EnvironmentError> {
        let alt_m = position.ellipsoidal_height_m;
        Isa.at(Altitude::GeometricMsl(alt_m))
            .map(|air| air.pressure_pa)
            .map_err(|source| EnvironmentError::Atmosphere { alt_m, source })
    }
}

#[cfg(test)]
mod tests;
