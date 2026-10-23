//! The world's magnetic field: one model, one epoch, so every simulated
//! magnetometer and every flight controller's declination agree.

pub use world_geomag::{GeomagModel, MagneticField, WMM2025};

use crate::error::WorldError;
use crate::position::world_position;

/// The date a world's magnetic field is evaluated at.
///
/// A world fixes its epoch when it starts, so a run is deterministic: the
/// field does not drift with the wall clock during a session.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MagneticEpoch {
    decimal_year: f64,
}

impl MagneticEpoch {
    /// The epoch at a decimal year, such as 2026.5.
    pub fn from_decimal_year(decimal_year: f64) -> Self {
        Self { decimal_year }
    }

    /// The epoch at a Unix time, seconds.
    pub fn from_unix_seconds(unix_seconds: i64) -> Self {
        Self::from_decimal_year(world_core::DecimalYear::from_unix_seconds(unix_seconds).value())
    }

    /// The epoch as a decimal year.
    pub fn decimal_year(self) -> f64 {
        self.decimal_year
    }
}

/// The main magnetic field at a point at `epoch`, in north-east-down
/// components, from the one model every world shares. `height_m` is a
/// world (MSL) height; see [`crate::world_position`] for how it meets the
/// ellipsoid. A host uses this to turn a vehicle's magnetic heading true
/// without a simulated world around it.
///
/// # Errors
///
/// Returns [`WorldError::MagneticField`] for a point outside the model's
/// domain or an epoch outside its validity.
pub fn magnetic_field_at(
    latitude_deg: f64,
    longitude_deg: f64,
    height_m: f64,
    epoch: MagneticEpoch,
) -> Result<MagneticField, WorldError> {
    let decimal_year = epoch.decimal_year();
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

#[cfg(test)]
mod tests;
