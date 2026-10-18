//! A local tangent plane around a geodetic origin.
//!
//! The flat-earth approximation with WGS-84 radii of curvature at the
//! origin latitude is exact to centimetres over the few hundred metres a
//! simulated quad covers.

/// WGS-84 semi-major axis, m.
const SEMI_MAJOR: f64 = 6_378_137.0;
/// WGS-84 first eccentricity squared.
const ECCENTRICITY_SQ: f64 = 6.694_379_990_14e-3;

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

impl Origin {
    /// Metres per radian of latitude and of longitude at the origin.
    fn radii(&self) -> (f64, f64) {
        let lat = self.lat_deg.to_radians();
        let s = lat.sin();
        let w = (1.0 - ECCENTRICITY_SQ * s * s).sqrt();
        let meridian = SEMI_MAJOR * (1.0 - ECCENTRICITY_SQ) / (w * w * w);
        let prime_vertical = SEMI_MAJOR / w;
        (meridian, prime_vertical * lat.cos())
    }

    /// Latitude, longitude (degrees), and altitude (m) of a point `north`,
    /// `east`, `down` metres from the origin.
    #[must_use]
    pub fn to_geodetic(&self, north: f64, east: f64, down: f64) -> (f64, f64, f64) {
        let (per_rad_lat, per_rad_lon) = self.radii();
        (
            self.lat_deg + (north / per_rad_lat).to_degrees(),
            self.lon_deg + (east / per_rad_lon).to_degrees(),
            self.alt_m - down,
        )
    }

    /// North, east, down metres of a geodetic point from the origin.
    #[must_use]
    pub fn to_local(&self, lat_deg: f64, lon_deg: f64, alt_m: f64) -> (f64, f64, f64) {
        let (per_rad_lat, per_rad_lon) = self.radii();
        (
            (lat_deg - self.lat_deg).to_radians() * per_rad_lat,
            (lon_deg - self.lon_deg).to_radians() * per_rad_lon,
            self.alt_m - alt_m,
        )
    }
}

#[cfg(test)]
mod tests;
