//! The world's magnetic field: one model, one epoch, so every simulated
//! magnetometer and every flight controller's declination agree.

pub use world_geomag::{GeomagModel, MagneticField, WMM2025};

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

#[cfg(test)]
mod tests;
