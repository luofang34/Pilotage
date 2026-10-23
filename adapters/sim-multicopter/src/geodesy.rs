//! The local frame around the launch point and the attitude convention.
//!
//! The horizontal frame is a plane tangent to the WGS-84 ellipsoid at the
//! launch point, with north, east, and down axes (NED). The attitude
//! quaternion uses the convention of the DJI adapter: aerospace yaw, pitch,
//! roll (ZYX), body x forward, y right, z down, world NED.

/// WGS-84 semi-major axis, metres.
const SEMI_MAJOR_M: f64 = 6_378_137.0;
/// WGS-84 first eccentricity squared.
const ECCENTRICITY_SQ: f64 = 6.694_379_990_14e-3;

/// A WGS-84 launch point: latitude and longitude in degrees, and the
/// altitude of the flat ground in metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LaunchPoint {
    /// Latitude, degrees north.
    pub lat_deg: f64,
    /// Longitude, degrees east.
    pub lon_deg: f64,
    /// Ground altitude, metres. The simulated ground is flat at this altitude.
    pub alt_m: f64,
}

/// A plane tangent to the WGS-84 ellipsoid at one launch point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalFrame {
    lat_deg: f64,
    lon_deg: f64,
    metres_per_rad_lat: f64,
    metres_per_rad_lon: f64,
}

impl LocalFrame {
    /// The tangent plane at `lat_deg`, `lon_deg`.
    #[must_use]
    pub fn anchored_at(lat_deg: f64, lon_deg: f64) -> Self {
        let lat = lat_deg.to_radians();
        let s = lat.sin();
        let w = (1.0 - ECCENTRICITY_SQ * s * s).sqrt();
        Self {
            lat_deg,
            lon_deg,
            metres_per_rad_lat: SEMI_MAJOR_M * (1.0 - ECCENTRICITY_SQ) / (w * w * w),
            metres_per_rad_lon: SEMI_MAJOR_M / w * lat.cos(),
        }
    }

    /// North and east metres of a point from the anchor.
    #[must_use]
    pub fn north_east(&self, lat_deg: f64, lon_deg: f64) -> [f64; 2] {
        [
            (lat_deg - self.lat_deg).to_radians() * self.metres_per_rad_lat,
            (lon_deg - self.lon_deg).to_radians() * self.metres_per_rad_lon,
        ]
    }

    /// Latitude and longitude, degrees, of a point north and east of the anchor.
    #[must_use]
    pub fn lat_lon(&self, north_m: f64, east_m: f64) -> [f64; 2] {
        [
            self.lat_deg + (north_m / self.metres_per_rad_lat).to_degrees(),
            self.lon_deg + (east_m / self.metres_per_rad_lon).to_degrees(),
        ]
    }
}

/// The body-FRD to world-NED quaternion (w, x, y, z) of roll, pitch, and
/// yaw applied in the yaw-pitch-roll order.
#[must_use]
pub fn quaternion_from_euler(roll_rad: f64, pitch_rad: f64, yaw_rad: f64) -> [f64; 4] {
    let (sr, cr) = (roll_rad / 2.0).sin_cos();
    let (sp, cp) = (pitch_rad / 2.0).sin_cos();
    let (sy, cy) = (yaw_rad / 2.0).sin_cos();
    [
        cr * cp * cy + sr * sp * sy,
        sr * cp * cy - cr * sp * sy,
        cr * sp * cy + sr * cp * sy,
        cr * cp * sy - sr * sp * cy,
    ]
}

/// Rotates a body-frame vector into the world frame with quaternion `q`.
#[must_use]
pub fn rotate(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
    let [w, x, y, z] = q;
    // t = 2 (q_vec x v); v' = v + w t + q_vec x t
    let t = [
        2.0 * (y * v[2] - z * v[1]),
        2.0 * (z * v[0] - x * v[2]),
        2.0 * (x * v[1] - y * v[0]),
    ];
    [
        v[0] + w * t[0] + (y * t[2] - z * t[1]),
        v[1] + w * t[1] + (z * t[0] - x * t[2]),
        v[2] + w * t[2] + (x * t[1] - y * t[0]),
    ]
}

/// Converts an `f64` quaternion to the `f32` telemetry representation.
#[must_use]
pub fn to_f32(q: [f64; 4]) -> [f32; 4] {
    q.map(|component| component as f32)
}

#[cfg(test)]
mod tests;
