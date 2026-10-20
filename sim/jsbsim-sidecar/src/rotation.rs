//! Attitude conventions: body FRD (x forward, y right, z down) and the
//! local NED frame, related by aerospace Z-Y-X Euler angles.

/// A unit quaternion `(w, x, y, z)` rotating body FRD vectors into NED.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quaternion {
    /// Scalar part.
    pub w: f64,
    /// Vector part, x.
    pub x: f64,
    /// Vector part, y.
    pub y: f64,
    /// Vector part, z.
    pub z: f64,
}

/// Roll, pitch, and yaw, radians, Z-Y-X order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Euler {
    /// Roll about body x.
    pub roll: f64,
    /// Pitch about body y.
    pub pitch: f64,
    /// Yaw about NED z, from true north.
    pub yaw: f64,
}

impl Euler {
    /// The body-to-NED quaternion.
    pub fn quaternion(self) -> Quaternion {
        let (sr, cr) = (self.roll / 2.0).sin_cos();
        let (sp, cp) = (self.pitch / 2.0).sin_cos();
        let (sy, cy) = (self.yaw / 2.0).sin_cos();
        Quaternion {
            w: cr * cp * cy + sr * sp * sy,
            x: sr * cp * cy - cr * sp * sy,
            y: cr * sp * cy + sr * cp * sy,
            z: cr * cp * sy - sr * sp * cy,
        }
    }

    /// A NED vector expressed in body FRD axes.
    pub fn ned_to_body(self, ned: [f64; 3]) -> [f64; 3] {
        let (sr, cr) = self.roll.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let (sy, cy) = self.yaw.sin_cos();
        let [n, e, d] = ned;
        [
            cp * cy * n + cp * sy * e - sp * d,
            (sr * sp * cy - cr * sy) * n + (sr * sp * sy + cr * cy) * e + sr * cp * d,
            (cr * sp * cy + sr * sy) * n + (cr * sp * sy - sr * cy) * e + cr * cp * d,
        ]
    }

    /// A body FRD vector expressed in NED axes.
    pub fn body_to_ned(self, body: [f64; 3]) -> [f64; 3] {
        let (sr, cr) = self.roll.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let (sy, cy) = self.yaw.sin_cos();
        let [x, y, z] = body;
        [
            cp * cy * x + (sr * sp * cy - cr * sy) * y + (cr * sp * cy + sr * sy) * z,
            cp * sy * x + (sr * sp * sy + cr * cy) * y + (cr * sp * sy - sr * cy) * z,
            -sp * x + sr * cp * y + cr * cp * z,
        ]
    }
}

#[cfg(test)]
mod tests;
