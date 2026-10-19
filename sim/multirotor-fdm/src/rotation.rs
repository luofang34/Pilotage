//! The small vector and quaternion algebra the model needs.

use std::ops::{Add, Mul, Neg, Sub};

/// A 3-vector.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec3 {
    /// First component.
    pub x: f64,
    /// Second component.
    pub y: f64,
    /// Third component.
    pub z: f64,
}

impl Vec3 {
    /// The zero vector.
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);

    /// A vector from its components.
    #[must_use]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// The dot product.
    #[must_use]
    pub fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    /// The cross product.
    #[must_use]
    pub fn cross(self, other: Self) -> Self {
        Self::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    /// The Euclidean length.
    #[must_use]
    pub fn norm(self) -> f64 {
        self.dot(self).sqrt()
    }
}

impl Add for Vec3 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl Sub for Vec3 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl Neg for Vec3 {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y, -self.z)
    }
}

impl Mul<f64> for Vec3 {
    type Output = Self;
    fn mul(self, rhs: f64) -> Self {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

/// A unit quaternion `w + xi + yj + zk` rotating body vectors into the
/// reference frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quat {
    /// Scalar part.
    pub w: f64,
    /// `i` part.
    pub x: f64,
    /// `j` part.
    pub y: f64,
    /// `k` part.
    pub z: f64,
}

impl Default for Quat {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Quat {
    /// No rotation.
    pub const IDENTITY: Self = Self {
        w: 1.0,
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    /// The rotation of `angle` radians about the unit `axis`.
    #[must_use]
    pub fn from_axis_angle(axis: Vec3, angle: f64) -> Self {
        let (s, c) = (angle / 2.0).sin_cos();
        Self {
            w: c,
            x: axis.x * s,
            y: axis.y * s,
            z: axis.z * s,
        }
    }

    /// The aerospace Z-Y-X rotation: yaw, then pitch, then roll (radians).
    #[must_use]
    pub fn from_euler(roll: f64, pitch: f64, yaw: f64) -> Self {
        Self::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), yaw)
            * Self::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), pitch)
            * Self::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), roll)
    }

    /// Roll, pitch, and yaw (radians) of the Z-Y-X decomposition.
    #[must_use]
    pub fn to_euler(self) -> (f64, f64, f64) {
        let Self { w, x, y, z } = self;
        let roll = (2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y));
        let pitch = (2.0 * (w * y - z * x)).clamp(-1.0, 1.0).asin();
        let yaw = (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z));
        (roll, pitch, yaw)
    }

    /// The inverse rotation.
    #[must_use]
    pub fn conjugate(self) -> Self {
        Self {
            w: self.w,
            x: -self.x,
            y: -self.y,
            z: -self.z,
        }
    }

    /// The same rotation with unit norm restored.
    #[must_use]
    pub fn normalized(self) -> Self {
        let n = (self.w * self.w + self.x * self.x + self.y * self.y + self.z * self.z).sqrt();
        if n <= f64::EPSILON {
            return Self::IDENTITY;
        }
        Self {
            w: self.w / n,
            x: self.x / n,
            y: self.y / n,
            z: self.z / n,
        }
    }

    /// Rotates `v` from the body frame into the reference frame.
    #[must_use]
    pub fn rotate(self, v: Vec3) -> Vec3 {
        let u = Vec3::new(self.x, self.y, self.z);
        let t = u.cross(v) * 2.0;
        v + t * self.w + u.cross(t)
    }

    /// Rotates `v` from the reference frame into the body frame.
    #[must_use]
    pub fn rotate_inverse(self, v: Vec3) -> Vec3 {
        self.conjugate().rotate(v)
    }

    /// Advances the attitude by body rate `w` (rad/s) over `dt` seconds.
    #[must_use]
    pub fn integrate(self, w: Vec3, dt: f64) -> Self {
        let angle = w.norm() * dt;
        if angle <= f64::EPSILON {
            return self;
        }
        (self * Self::from_axis_angle(w * (1.0 / w.norm()), angle)).normalized()
    }
}

impl Mul for Quat {
    type Output = Self;
    fn mul(self, r: Self) -> Self {
        Self {
            w: self.w * r.w - self.x * r.x - self.y * r.y - self.z * r.z,
            x: self.w * r.x + self.x * r.w + self.y * r.z - self.z * r.y,
            y: self.w * r.y - self.x * r.z + self.y * r.w + self.z * r.x,
            z: self.w * r.z + self.x * r.y - self.y * r.x + self.z * r.w,
        }
    }
}
