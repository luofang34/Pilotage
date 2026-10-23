//! A deterministic point-mass multicopter in the local NED frame.
//!
//! The velocity and the yaw rate follow the commanded values with a
//! first-order response. The tilt is derived from the horizontal
//! acceleration, so the attitude looks like a multicopter that leans into
//! its acceleration. The ground is flat at height zero.

use crate::config::Limits;

/// Standard gravity, metres per second squared.
const GRAVITY_MPS2: f64 = 9.806_65;

/// A body-frame velocity command: forward, right, and down metres per
/// second, and the yaw rate in radians per second (positive clockwise
/// seen from above).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BodyVelocity {
    /// Forward speed, metres per second.
    pub forward: f64,
    /// Right speed, metres per second.
    pub right: f64,
    /// Down speed, metres per second. Negative climbs.
    pub down: f64,
    /// Yaw rate, radians per second.
    pub yaw_rate: f64,
}

impl BodyVelocity {
    /// The command clamped to `limits`; NaN becomes zero. Returns whether
    /// a component changed.
    #[must_use]
    pub fn clamped(self, limits: &Limits) -> (Self, bool) {
        let clamp = |value: f64, bound: f64| {
            if value.is_nan() {
                0.0
            } else {
                value.clamp(-bound, bound)
            }
        };
        let horizontal = self.forward.hypot(self.right);
        let scale = if horizontal > limits.horizontal_mps && horizontal.is_finite() {
            limits.horizontal_mps / horizontal
        } else {
            1.0
        };
        let clamped = Self {
            forward: clamp(self.forward * scale, limits.horizontal_mps),
            right: clamp(self.right * scale, limits.horizontal_mps),
            down: clamp(self.down, limits.vertical_mps),
            yaw_rate: clamp(self.yaw_rate, limits.yaw_rate_rps),
        };
        let changed = clamped != self;
        (clamped, changed)
    }
}

/// The point-mass state in the local NED frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Kinematics {
    /// Position north, east, down from the launch point, metres.
    pub pos_ned_m: [f64; 3],
    /// Velocity north, east, down, metres per second.
    pub vel_ned_mps: [f64; 3],
    /// Heading, radians from true north, positive clockwise, in (-pi, pi].
    pub yaw_rad: f64,
    /// Yaw rate, radians per second.
    pub yaw_rate_rps: f64,
    /// Acceleration of the last step, metres per second squared, NED.
    pub accel_ned_mps2: [f64; 3],
}

impl Kinematics {
    /// A vehicle at rest on the ground at the launch point.
    #[must_use]
    pub fn at_rest(yaw_rad: f64) -> Self {
        Self {
            yaw_rad,
            ..Self::default()
        }
    }

    /// Height above the launch ground, metres.
    #[must_use]
    pub fn height_m(&self) -> f64 {
        -self.pos_ned_m[2]
    }

    /// Horizontal ground speed, metres per second.
    #[must_use]
    pub fn ground_speed_mps(&self) -> f64 {
        self.vel_ned_mps[0].hypot(self.vel_ned_mps[1])
    }

    /// Advances the state by `dt_s` towards `command`. The ground stops a
    /// descent at height zero. Returns true when the vehicle touched the
    /// ground in this step.
    pub fn step(&mut self, command: BodyVelocity, limits: &Limits, dt_s: f64) -> bool {
        let (command, _) = command.clamped(limits);
        let (sin, cos) = self.yaw_rad.sin_cos();
        let target = [
            command.forward * cos - command.right * sin,
            command.forward * sin + command.right * cos,
            command.down,
        ];
        let horizontal_gain = response_gain(dt_s, limits.velocity_time_constant_s);
        let yaw_gain = response_gain(dt_s, limits.yaw_time_constant_s);
        let previous = self.vel_ned_mps;
        for (axis, wanted) in target.iter().enumerate() {
            self.vel_ned_mps[axis] += (wanted - self.vel_ned_mps[axis]) * horizontal_gain;
        }
        self.yaw_rate_rps += (command.yaw_rate - self.yaw_rate_rps) * yaw_gain;
        for (axis, before) in previous.iter().enumerate() {
            self.pos_ned_m[axis] += self.vel_ned_mps[axis] * dt_s;
            self.accel_ned_mps2[axis] = (self.vel_ned_mps[axis] - before) / dt_s;
        }
        self.yaw_rad = wrap_pi(self.yaw_rad + self.yaw_rate_rps * dt_s);
        if self.pos_ned_m[2] >= 0.0 && self.vel_ned_mps[2] >= 0.0 {
            let touched = self.pos_ned_m[2] > 0.0 || self.vel_ned_mps[2] > 0.0;
            self.pos_ned_m[2] = 0.0;
            self.vel_ned_mps[2] = 0.0;
            return touched;
        }
        false
    }

    /// Stops all motion on the ground: the vehicle does not slide or turn.
    pub fn hold_on_ground(&mut self) {
        self.pos_ned_m[2] = 0.0;
        self.vel_ned_mps = [0.0; 3];
        self.accel_ned_mps2 = [0.0; 3];
        self.yaw_rate_rps = 0.0;
    }

    /// Roll and pitch, radians, of a multicopter that holds the present
    /// horizontal acceleration: it pitches nose-down to accelerate forward
    /// and rolls right to accelerate right.
    #[must_use]
    pub fn tilt_rad(&self) -> (f64, f64) {
        let (sin, cos) = self.yaw_rad.sin_cos();
        let [north, east, _] = self.accel_ned_mps2;
        let forward = north * cos + east * sin;
        let right = -north * sin + east * cos;
        let roll = right.atan2(GRAVITY_MPS2);
        let pitch = (-forward).atan2(GRAVITY_MPS2);
        (roll, pitch)
    }
}

/// The exact first-order step gain for one interval.
fn response_gain(dt_s: f64, time_constant_s: f64) -> f64 {
    if time_constant_s <= 0.0 {
        1.0
    } else {
        1.0 - (-dt_s / time_constant_s).exp()
    }
}

/// Wraps an angle into (-pi, pi].
#[must_use]
pub fn wrap_pi(angle: f64) -> f64 {
    let wrapped = angle.rem_euclid(core::f64::consts::TAU);
    if wrapped > core::f64::consts::PI {
        wrapped - core::f64::consts::TAU
    } else {
        wrapped
    }
}

#[cfg(test)]
mod tests;
