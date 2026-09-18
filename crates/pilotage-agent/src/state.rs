//! Vehicle state for control, and truth for the verdict.
//!
//! The two are separate types from separate sources. The executor flies on the
//! operational estimate, as a real vehicle must. The verifier judges on truth,
//! so an estimator error cannot pass a flight that missed its target.

/// The state that the executor and guidance fly on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VehicleState {
    /// Metres north of the launch point.
    pub north_m: f64,
    /// Metres east of the launch point.
    pub east_m: f64,
    /// Velocity north, in metres per second.
    pub vel_north_mps: f64,
    /// Velocity east, in metres per second.
    pub vel_east_mps: f64,
    /// Height above the launch point, or `None` for a planar vehicle.
    pub height_m: Option<f64>,
    /// Climb rate in metres per second, positive up.
    pub climb_mps: f64,
    /// Heading in radians from north toward east.
    pub yaw_rad: f64,
    /// Armed state as the flight controller reports it, when it does.
    pub armed: Option<bool>,
}

/// Truth, for the verdict only.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TruthState {
    /// Metres north of the launch point.
    pub north_m: f64,
    /// Metres east of the launch point.
    pub east_m: f64,
    /// Height above the launch point, or `None` for a planar vehicle.
    pub height_m: Option<f64>,
    /// Heading in radians from north toward east, when truth has attitude.
    pub yaw_rad: Option<f64>,
}

/// Yaw of a body-to-NED quaternion, in radians from north toward east.
#[must_use]
pub fn yaw_of_quaternion(w: f64, x: f64, y: f64, z: f64) -> f64 {
    (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z))
}

#[cfg(test)]
mod tests {
    use core::f64::consts::{FRAC_PI_2, FRAC_PI_4};

    use super::yaw_of_quaternion;

    #[test]
    fn a_quarter_turn_about_down_reads_as_east() {
        let yaw = yaw_of_quaternion(FRAC_PI_4.cos(), 0.0, 0.0, FRAC_PI_4.sin());
        assert!((yaw - FRAC_PI_2).abs() < 1e-9);
    }

    #[test]
    fn the_identity_quaternion_reads_as_north() {
        assert!(yaw_of_quaternion(1.0, 0.0, 0.0, 0.0).abs() < 1e-12);
    }
}
