//! Deterministic coordinate and vertical-datum conversion.
//!
//! The datum vocabulary is [`pilotage_geo`]'s ([`HorizontalDatum`],
//! [`VerticalDatum`]); this module only converts between values expressed in it.
//! Both conversions are pure, closed-form, and deterministic, so the same input
//! always yields the same output bits under a fixed toolchain.
//!
//! Horizontal conversion is a fixed per-datum translation composed through
//! WGS-84, a simulator stand-in for a real datum shift that keeps a regular grid
//! regular (a translation maps a lattice to a lattice). Vertical conversion is
//! ellipsoid-versus-MSL through a closed-form synthetic geoid separation. Any
//! datum this build does not convert is refused, never approximated. These are
//! engineering stand-ins, not surveyed transforms.

#[cfg(test)]
mod tests;

use pilotage_geo::{HorizontalDatum, VerticalDatum, wrap_longitude_deg};

use crate::error::BuildError;

/// The fixed offset (degrees) added to a datum's coordinates to reach WGS-84, as
/// `(delta_lat, delta_lon)`. WGS-84 is the reference, so its offset is zero.
/// These are small synthetic constants, not surveyed parameters.
const fn offset_to_wgs84_deg(datum: HorizontalDatum) -> (f64, f64) {
    match datum {
        HorizontalDatum::Wgs84 => (0.0, 0.0),
        HorizontalDatum::Nad83 => (0.000_001_0, -0.000_001_5),
        HorizontalDatum::Itrf2014 => (0.000_000_4, 0.000_000_2),
        HorizontalDatum::Unknown => (0.0, 0.0),
    }
}

/// Converts a coordinate from `from` to `to`, returning `(lat_deg, lon_deg)` in
/// the target datum with longitude normalized to `[-180, 180)`.
///
/// # Errors
///
/// [`BuildError::UnsupportedDatumConversion`] when either datum is `Unknown` —
/// an unknown frame is refused rather than assumed to coincide with WGS-84.
pub fn convert_horizontal(
    lat_deg: f64,
    lon_deg: f64,
    from: HorizontalDatum,
    to: HorizontalDatum,
) -> Result<(f64, f64), BuildError> {
    if from == HorizontalDatum::Unknown || to == HorizontalDatum::Unknown {
        return Err(BuildError::UnsupportedDatumConversion {
            from: from.to_u8(),
            to: to.to_u8(),
            axis: "horizontal",
        });
    }
    let (from_dlat, from_dlon) = offset_to_wgs84_deg(from);
    let (to_dlat, to_dlon) = offset_to_wgs84_deg(to);
    let lat = lat_deg + (from_dlat - to_dlat);
    let lon = lon_deg + (from_dlon - to_dlon);
    Ok((lat, wrap_longitude_deg(lon)))
}

/// A closed-form synthetic geoid separation (meters) at `lat_deg`/`lon_deg`: the
/// height of the geoid above the ellipsoid. Deterministic and smooth; a
/// simulator stand-in for a real geoid model, never a surveyed separation.
#[must_use]
pub fn geoid_separation_m(lat_deg: f64, lon_deg: f64) -> f64 {
    let lat = lat_deg.to_radians();
    let lon = lon_deg.to_radians();
    30.0 * lat.sin() + 10.0 * (2.0 * lon).cos()
}

/// Converts a height from vertical datum `from` to `to` at `lat_deg`/`lon_deg`.
/// Only ellipsoid-versus-MSL is converted, through [`geoid_separation_m`]
/// (`h_ellipsoid = H_msl + N`); a same-datum conversion is the identity.
///
/// # Errors
///
/// [`BuildError::UnsupportedDatumConversion`] for any datum pair outside
/// ellipsoid/MSL — an unsupported vertical reference is refused, not
/// approximated.
pub fn convert_vertical(
    height_m: f64,
    from: VerticalDatum,
    to: VerticalDatum,
    lat_deg: f64,
    lon_deg: f64,
) -> Result<f64, BuildError> {
    if from == to {
        return Ok(height_m);
    }
    let separation = geoid_separation_m(lat_deg, lon_deg);
    match (from, to) {
        (VerticalDatum::Msl, VerticalDatum::Ellipsoid) => Ok(height_m + separation),
        (VerticalDatum::Ellipsoid, VerticalDatum::Msl) => Ok(height_m - separation),
        _ => Err(BuildError::UnsupportedDatumConversion {
            from: from.to_u8(),
            to: to.to_u8(),
            axis: "vertical",
        }),
    }
}
