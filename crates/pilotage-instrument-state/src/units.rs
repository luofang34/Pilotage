//! Unit conversions between the SI input state and cockpit display units.
//!
//! Input state is SI (meters, meters/second, radians); panels display
//! aviation units (feet, knots, feet/minute, degrees). Conversion happens
//! once, in [`crate::resolve`], never inside drawing code.

/// Meters/second to knots.
pub const MPS_TO_KT: f32 = 1.943_844_5;

/// Meters to feet.
pub const M_TO_FT: f32 = 3.280_84;

/// Meters/second to feet/minute.
pub const MPS_TO_FPM: f32 = 196.850_4;

/// Radians to degrees.
pub const RAD_TO_DEG: f32 = 57.295_78;

/// Normalizes an angle in degrees into `[0, 360)`.
pub fn wrap_deg_360(deg: f32) -> f32 {
    let r = libm::fmodf(deg, 360.0);
    if r < 0.0 { r + 360.0 } else { r }
}
