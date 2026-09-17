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
