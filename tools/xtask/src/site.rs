//! Where a fleet stands: a named airfield, or one an operator states, and
//! the layout that puts each vehicle on it.
//!
//! A site is one runway threshold. A vehicle that takes off from a runway
//! starts on the centreline there, facing down the runway; every other
//! vehicle starts on the grass to the left of the runway, clear of it, in
//! a row along it.

use crate::backend::SpawnPoint;
use crate::error::XtaskError;

/// An airfield the fleet stands on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Site {
    /// The name `--site` selects it by, or `custom` for a stated one.
    pub name: &'static str,
    /// Latitude of the runway threshold, degrees north (WGS84).
    pub latitude_deg: f64,
    /// Longitude of the runway threshold, degrees east (WGS84).
    pub longitude_deg: f64,
    /// Ground elevation at the threshold, metres above mean sea level.
    pub elevation_m: f64,
    /// The runway's true course from the threshold, degrees clockwise
    /// from north.
    pub heading_deg: f64,
}

/// Solberg-Hunterdon Airport (N51), Readington, New Jersey: the
/// south-west end of the asphalt of runway 04/22.
///
/// The threshold and the course come from the OpenStreetMap runway way
/// (© OpenStreetMap contributors, ODbL): its ends are 40.5771861 N
/// 74.7381918 W and 40.5843352 N 74.7328829 W, a true course of 029.5°.
/// The elevation is the AWS Terrain Tiles (Terrarium) sample at the
/// threshold, so a vehicle stands on the terrain a map draws there.
pub const N51: Site = Site {
    name: "n51",
    latitude_deg: 40.577_186_1,
    longitude_deg: -74.738_191_8,
    elevation_m: 56.3,
    heading_deg: 29.5,
};

/// PX4's default home in Zurich. It has no runway; its "runway" points
/// east, the way PX4's own worlds face their vehicles.
pub const ZURICH: Site = Site {
    name: "zurich",
    latitude_deg: 47.397_742,
    longitude_deg: 8.545_594,
    elevation_m: 488.0,
    heading_deg: 90.0,
};

/// The site a fleet stands on when `--site` is absent.
pub const DEFAULT_SITE: Site = N51;

/// Every named site, for `--site <name>`.
pub const NAMED_SITES: [Site; 2] = [N51, ZURICH];

/// How far apart two vehicles on the runway start, along it, m.
const RUNWAY_SPACING_M: f64 = 60.0;
/// How far apart two vehicles on the grass start, along the runway, m.
const FIELD_SPACING_M: f64 = 40.0;
/// How far left of the centreline the grass row stands, m. The N51
/// runway is 15 m wide; its taxiway and hangars are on its right side.
const FIELD_OFFSET_M: f64 = 40.0;
/// WGS84 semi-major axis, m, and first eccentricity squared.
const WGS84_A_M: f64 = 6_378_137.0;
const WGS84_E2: f64 = 6.694_379_990_14e-3;

/// Metres per degree of latitude and of longitude at `latitude_deg`, from
/// the WGS84 radii of curvature there: exact to centimetres over the few
/// hundred metres of a layout.
fn metres_per_degree(latitude_deg: f64) -> (f64, f64) {
    let phi = latitude_deg.to_radians();
    let w = (1.0 - WGS84_E2 * phi.sin().powi(2)).sqrt();
    let meridian = WGS84_A_M * (1.0 - WGS84_E2) / w.powi(3);
    let normal = WGS84_A_M / w;
    (meridian.to_radians(), (normal * phi.cos()).to_radians())
}

/// Where on a site a vehicle starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ground {
    /// On the runway centreline, for a vehicle that takes off along it.
    Runway,
    /// On the grass beside the runway, for one that lifts off in place.
    Field,
}

impl Site {
    /// Parses `--site`: a name from [`NAMED_SITES`], or
    /// `latitude_deg,longitude_deg,elevation_m,heading_deg`.
    ///
    /// # Errors
    ///
    /// Returns [`XtaskError::Usage`] for an unknown name, a malformed
    /// list, or a value out of range.
    pub fn parse(spec: &str) -> Result<Self, XtaskError> {
        if let Some(site) = NAMED_SITES.iter().find(|site| site.name == spec) {
            return Ok(*site);
        }
        let usage = |reason: &str| XtaskError::Usage {
            message: format!(
                "--site {spec:?}: {reason}; give one of {} or \
                 latitude_deg,longitude_deg,elevation_m,heading_deg",
                NAMED_SITES.map(|site| site.name).join(", ")
            ),
        };
        let values: Vec<f64> = spec
            .split(',')
            .map(|value| value.trim().parse::<f64>())
            .collect::<Result<_, _>>()
            .map_err(|_| usage("not a known site or a list of numbers"))?;
        let [latitude_deg, longitude_deg, elevation_m, heading_deg] = values[..] else {
            return Err(usage("a stated site has four values"));
        };
        if !values.iter().all(|value| value.is_finite()) {
            return Err(usage("every value must be finite"));
        }
        if latitude_deg.abs() > 85.0 || longitude_deg.abs() > 180.0 {
            return Err(usage("the position is off the map"));
        }
        if !(0.0..360.0).contains(&heading_deg) {
            return Err(usage("the heading must be 0 to 360 degrees"));
        }
        Ok(Self {
            name: "custom",
            latitude_deg,
            longitude_deg,
            elevation_m,
            heading_deg,
        })
    }

    /// Where the `index`-th vehicle (from 0) on `ground` starts. Every
    /// vehicle faces down the runway and stands at the site's elevation.
    pub fn spawn_point(&self, ground: Ground, index: usize) -> SpawnPoint {
        let step = index as f64;
        let (along_m, left_m) = match ground {
            Ground::Runway => (RUNWAY_SPACING_M * step, 0.0),
            Ground::Field => (FIELD_SPACING_M * (step + 1.0), FIELD_OFFSET_M),
        };
        let heading = self.heading_deg.to_radians();
        let north_m = along_m * heading.cos() + left_m * heading.sin();
        let east_m = along_m * heading.sin() - left_m * heading.cos();
        let (per_lat_degree, per_lon_degree) = metres_per_degree(self.latitude_deg);
        SpawnPoint {
            latitude_deg: self.latitude_deg + north_m / per_lat_degree,
            longitude_deg: self.longitude_deg + east_m / per_lon_degree,
            elevation_m: self.elevation_m,
            heading_deg: self.heading_deg,
        }
    }
}

#[cfg(test)]
mod tests;
