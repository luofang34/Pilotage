//! Coverage: the geographic region a package spans and the datum that region's
//! coordinates and heights are expressed in.
//!
//! The datum vocabulary is consumed from `pilotage-geo`
//! ([`pilotage_geo::HorizontalDatum`], [`pilotage_geo::VerticalDatum`], and
//! their realization/geoid identities) rather than re-minted here, so a package
//! and the synthetic-vision contract that consumes it can never disagree about
//! what a datum means. A query outside the box is a coverage exit, distinct
//! from a database fault.

use pilotage_geo::{
    DatumRealizationId, GeodeticPosition, GeoidModelId, HorizontalDatum, VerticalDatum,
};

/// The horizontal ground resolution a terrain-bearing package is sampled at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolution {
    /// Terrain post spacing, millimeters. Zero means unspecified.
    pub post_spacing_mm: u32,
}

/// A geographic bounding box in degrees, half-open on the upper edges so
/// adjacent boxes tile without overlap. Validated to be finite with
/// `min < max` on both axes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoverageBox {
    /// Southern latitude bound, degrees (inclusive).
    pub min_lat_deg: f64,
    /// Northern latitude bound, degrees (exclusive).
    pub max_lat_deg: f64,
    /// Western longitude bound, degrees (inclusive).
    pub min_lon_deg: f64,
    /// Eastern longitude bound, degrees (exclusive).
    pub max_lon_deg: f64,
}

impl CoverageBox {
    /// Whether every bound is finite and the box is non-degenerate.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.min_lat_deg.is_finite()
            && self.max_lat_deg.is_finite()
            && self.min_lon_deg.is_finite()
            && self.max_lon_deg.is_finite()
            && self.min_lat_deg < self.max_lat_deg
            && self.min_lon_deg < self.max_lon_deg
    }

    /// Whether `lat_deg`/`lon_deg` fall inside the box (inclusive lower,
    /// exclusive upper).
    #[must_use]
    pub fn contains_lat_lon(&self, lat_deg: f64, lon_deg: f64) -> bool {
        lat_deg >= self.min_lat_deg
            && lat_deg < self.max_lat_deg
            && lon_deg >= self.min_lon_deg
            && lon_deg < self.max_lon_deg
    }

    /// Whether a geodetic position lies inside the box.
    #[must_use]
    pub fn contains(&self, pos: &GeodeticPosition) -> bool {
        self.contains_lat_lon(pos.latitude_deg, pos.longitude_deg)
    }
}

/// The covered region together with the datum its coordinates and heights are
/// referenced to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coverage {
    /// The bounding region.
    pub region: CoverageBox,
    /// The horizontal datum of the region's coordinates.
    pub horizontal_datum: HorizontalDatum,
    /// The horizontal-datum realization; `UNDECLARED` for a datum that needs
    /// none.
    pub realization: DatumRealizationId,
    /// The vertical datum of the package's heights.
    pub vertical_datum: VerticalDatum,
    /// The geoid model behind a geometric-MSL vertical datum; `UNDECLARED`
    /// otherwise.
    pub geoid: GeoidModelId,
    /// The horizontal ground resolution.
    pub resolution: Resolution,
}
