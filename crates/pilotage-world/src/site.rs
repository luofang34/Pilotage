//! Where vehicles stand: a named airfield, or one an operator states, and
//! the layout that puts each vehicle on it.
//!
//! A site is one runway threshold. A vehicle that takes off from a runway
//! starts on the centreline there, facing down the runway; every other
//! vehicle starts on the grass to the left of the runway, clear of it, in
//! a row along it.

/// An airfield vehicles stand on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Site {
    /// The name a launcher selects it by, or `custom` for a stated one.
    pub name: &'static str,
    /// Latitude of the runway threshold, degrees north (WGS84).
    pub latitude_deg: f64,
    /// Longitude of the runway threshold, degrees east (WGS84).
    pub longitude_deg: f64,
    /// Declared ground elevation at the threshold, metres above mean sea
    /// level. A world with terrain coverage there replaces it with the
    /// terrain's own height (see [`crate::World::site_elevation_blocking`]).
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

/// The site vehicles stand on when none is selected.
pub const DEFAULT_SITE: Site = N51;

/// Every named site.
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
pub fn metres_per_degree(latitude_deg: f64) -> (f64, f64) {
    let phi = latitude_deg.to_radians();
    let w = libm::sqrt(1.0 - WGS84_E2 * libm::pow(libm::sin(phi), 2.0));
    let meridian = WGS84_A_M * (1.0 - WGS84_E2) / libm::pow(w, 3.0);
    let normal = WGS84_A_M / w;
    (
        meridian.to_radians(),
        (normal * libm::cos(phi)).to_radians(),
    )
}

/// Where on a site a vehicle starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ground {
    /// On the runway centreline, for a vehicle that takes off along it.
    Runway,
    /// On the grass beside the runway, for one that lifts off in place.
    Field,
}

/// Where one vehicle starts: a point on the WGS84 ellipsoid, the ground
/// height there, and the way the vehicle faces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Latitude, degrees north.
    pub latitude_deg: f64,
    /// Longitude, degrees east.
    pub longitude_deg: f64,
    /// Ground elevation, metres above mean sea level. The vehicle stands
    /// on the ground, so this is also its starting altitude.
    pub elevation_m: f64,
    /// True heading of the vehicle's nose, degrees clockwise from north.
    pub heading_deg: f64,
}

impl Site {
    /// The named site called `name`, if there is one.
    pub fn named(name: &str) -> Option<Self> {
        NAMED_SITES.iter().find(|site| site.name == name).copied()
    }

    /// Where the `index`-th vehicle (from 0) on `ground` starts. Every
    /// vehicle faces down the runway and stands at the site's declared
    /// elevation.
    pub fn placement(&self, ground: Ground, index: usize) -> Placement {
        let step = index as f64;
        let (along_m, left_m) = match ground {
            Ground::Runway => (RUNWAY_SPACING_M * step, 0.0),
            Ground::Field => (FIELD_SPACING_M * (step + 1.0), FIELD_OFFSET_M),
        };
        let heading = self.heading_deg.to_radians();
        let (sin, cos) = (libm::sin(heading), libm::cos(heading));
        let north_m = along_m * cos + left_m * sin;
        let east_m = along_m * sin - left_m * cos;
        let (per_lat_degree, per_lon_degree) = metres_per_degree(self.latitude_deg);
        Placement {
            latitude_deg: self.latitude_deg + north_m / per_lat_degree,
            longitude_deg: self.longitude_deg + east_m / per_lon_degree,
            elevation_m: self.elevation_m,
            heading_deg: self.heading_deg,
        }
    }
}

#[cfg(test)]
mod tests;
