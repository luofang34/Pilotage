//! Attitude quaternion and Euler extraction.

use libm::{asinf, atan2f};

/// A unit quaternion rotating body (FRD) into world (NED), the convention
/// Aviate's state estimate uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quat {
    /// Scalar part.
    pub w: f32,
    /// Vector x (roll axis).
    pub x: f32,
    /// Vector y (pitch axis).
    pub y: f32,
    /// Vector z (yaw axis).
    pub z: f32,
}

impl Quat {
    /// The identity rotation (level, north).
    pub const IDENTITY: Self = Self {
        w: 1.0,
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    /// Aerospace ZYX Euler angles `(roll, pitch, yaw)` in radians.
    ///
    /// Roll is positive right-wing-down, pitch positive nose-up, yaw
    /// positive clockwise from north. Pitch is clamped into ±90° so a
    /// slightly denormalized quaternion cannot produce NaN.
    pub fn to_euler(self) -> (f32, f32, f32) {
        let (w, x, y, z) = (self.w, self.x, self.y, self.z);
        let roll = atan2f(2.0 * (w * x + y * z), 1.0 - 2.0 * (x * x + y * y));
        let sinp = (2.0 * (w * y - z * x)).clamp(-1.0, 1.0);
        let pitch = asinf(sinp);
        let yaw = atan2f(2.0 * (w * z + x * y), 1.0 - 2.0 * (y * y + z * z));
        (roll, pitch, yaw)
    }
}

#[cfg(test)]
mod tests;
