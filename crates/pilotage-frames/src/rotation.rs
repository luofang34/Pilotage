//! The single SO(3) rotation kernel (FRAME-01).
//!
//! Unit quaternions are the only three-dimensional rotation
//! implementation in the workspace; every frame transform and every
//! attitude representation composes through this type. Consumers that
//! need Euler angles or a down vector derive them explicitly — the
//! canonical state is never reduced to them.

use libm::{asinf, atan2f, sqrtf};

/// A unit quaternion rotating one frame's coordinates into another's.
/// Which frames those are is carried by the wrapping type
/// ([`crate::Tagged`], [`crate::FrameTransform`]) — a bare `Quat` has no
/// implicit frame pairing. The aircraft convention (body FRD → world
/// NED) is one tagged use among several.
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

impl Quat {
    /// Hamilton product `self ⊗ rhs`: applying `rhs` first, then `self`.
    pub fn compose(self, rhs: Self) -> Self {
        Self {
            w: self.w * rhs.w - self.x * rhs.x - self.y * rhs.y - self.z * rhs.z,
            x: self.w * rhs.x + self.x * rhs.w + self.y * rhs.z - self.z * rhs.y,
            y: self.w * rhs.y - self.x * rhs.z + self.y * rhs.w + self.z * rhs.x,
            z: self.w * rhs.z + self.x * rhs.y - self.y * rhs.x + self.z * rhs.w,
        }
    }

    /// The inverse rotation (conjugate; valid for unit quaternions).
    pub fn inverse(self) -> Self {
        Self {
            w: self.w,
            x: -self.x,
            y: -self.y,
            z: -self.z,
        }
    }

    /// Rotates a vector, promoting to f64 so large translations (ECEF
    /// magnitudes) do not lose meters to f32 rounding.
    pub fn rotate(self, v: [f64; 3]) -> [f64; 3] {
        let (w, x, y, z) = (
            f64::from(self.w),
            f64::from(self.x),
            f64::from(self.y),
            f64::from(self.z),
        );
        // v' = v + 2w(q×v) + 2q×(q×v), the allocation-free sandwich.
        let qv = [x, y, z];
        let c1 = cross(qv, v);
        let c2 = cross(qv, [c1[0], c1[1], c1[2]]);
        [
            v[0] + 2.0 * (w * c1[0] + c2[0]),
            v[1] + 2.0 * (w * c1[1] + c2[1]),
            v[2] + 2.0 * (w * c1[2] + c2[2]),
        ]
    }

    /// Renormalizes a quaternion whose norm drifted within `tolerance`
    /// of unity; zero, gross, or non-finite norms are rejected — the
    /// kernel never repairs a rotation that is not one.
    pub fn renormalized(self, tolerance: f32) -> Result<Self, NotARotation> {
        let finite =
            self.w.is_finite() && self.x.is_finite() && self.y.is_finite() && self.z.is_finite();
        if !finite {
            return Err(NotARotation);
        }
        let norm = sqrtf(self.w * self.w + self.x * self.x + self.y * self.y + self.z * self.z);
        if !norm.is_finite() || (norm - 1.0).abs() > tolerance {
            return Err(NotARotation);
        }
        Ok(Self {
            w: self.w / norm,
            x: self.x / norm,
            y: self.y / norm,
            z: self.z / norm,
        })
    }
}

/// The value offered as a rotation has zero, gross, or non-finite norm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotARotation;

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[cfg(test)]
mod tests;
