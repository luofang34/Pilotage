//! The local NED frame the adapter publishes positions in.
//!
//! Betaflight reports a geodetic fix and an altitude above its own
//! reference, never a local position. The adapter anchors a tangent plane
//! at the first usable fix of an attachment and measures north and east
//! from it; down is the negated Betaflight altitude. Over the few hundred
//! metres a quad covers, the flat-earth approximation with WGS-84 radii of
//! curvature at the anchor latitude is exact to centimetres.

/// WGS-84 semi-major axis, m.
const SEMI_MAJOR: f64 = 6_378_137.0;
/// WGS-84 first eccentricity squared.
const ECCENTRICITY_SQ: f64 = 6.694_379_990_14e-3;

/// A tangent plane anchored at one geodetic point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LocalFrame {
    lat_deg: f64,
    lon_deg: f64,
    metres_per_rad_lat: f64,
    metres_per_rad_lon: f64,
}

impl LocalFrame {
    /// The plane tangent at `lat_deg`, `lon_deg`.
    pub(crate) fn anchored_at(lat_deg: f64, lon_deg: f64) -> Self {
        let lat = lat_deg.to_radians();
        let s = lat.sin();
        let w = (1.0 - ECCENTRICITY_SQ * s * s).sqrt();
        Self {
            lat_deg,
            lon_deg,
            metres_per_rad_lat: SEMI_MAJOR * (1.0 - ECCENTRICITY_SQ) / (w * w * w),
            metres_per_rad_lon: SEMI_MAJOR / w * lat.cos(),
        }
    }

    /// North and east metres of a point from the anchor.
    pub(crate) fn north_east(&self, lat_deg: f64, lon_deg: f64) -> [f64; 2] {
        [
            (lat_deg - self.lat_deg).to_radians() * self.metres_per_rad_lat,
            (lon_deg - self.lon_deg).to_radians() * self.metres_per_rad_lon,
        ]
    }
}

#[cfg(test)]
mod tests;
