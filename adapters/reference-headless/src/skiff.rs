//! The `skiff`: a 2D planar reference vehicle model.
//!
//! All trigonometry goes through `libm` rather than `std::f64`'s intrinsics:
//! `std`'s `sin`/`cos` on some targets defer to the platform's system `libm`,
//! whose last-bit rounding can differ across operating systems and
//! toolchains. The golden trajectory tests in this crate assert exact `f64`
//! equality, so a portable, statically-linked `libm` is what makes those
//! assertions reproducible on every machine that runs this workspace's CI.
use serde::{Deserialize, Serialize};

/// Longitudinal acceleration applied at full throttle, in units/s^2.
pub const MAX_ACCEL: f64 = 4.0;
/// Linear drag coefficient applied to speed each tick.
pub const DRAG: f64 = 0.5;
/// Yaw rate applied at full steering deflection, in radians/s.
pub const YAW_RATE: f64 = 1.5;
/// Fixed simulation tick duration, in seconds.
pub const DT_SECONDS: f64 = 0.010;

/// The skiff's full dynamic state.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SkiffState {
    /// Planar position, `[x, y]`.
    pub pos: [f64; 2],
    /// Heading in radians.
    pub heading: f64,
    /// Scalar forward speed.
    pub speed: f64,
}

impl SkiffState {
    /// Constructs a skiff at the origin, at rest, facing along +x.
    #[must_use]
    pub const fn at_rest() -> Self {
        Self {
            pos: [0.0, 0.0],
            heading: 0.0,
            speed: 0.0,
        }
    }

    /// Advances the state by one fixed tick under the given controls.
    ///
    /// `throttle` and `steering` are in `[-1.0, 1.0]`; callers are
    /// responsible for clamping upstream if a wider range is possible.
    #[must_use]
    pub fn step(self, throttle: f64, steering: f64) -> Self {
        let speed = self.speed + (throttle * MAX_ACCEL - DRAG * self.speed) * DT_SECONDS;
        let heading = self.heading + steering * YAW_RATE * DT_SECONDS;
        let dx = self.speed * libm::cos(self.heading) * DT_SECONDS;
        let dy = self.speed * libm::sin(self.heading) * DT_SECONDS;
        Self {
            pos: [self.pos[0] + dx, self.pos[1] + dy],
            heading,
            speed,
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::SkiffState;

    #[test]
    fn at_rest_stays_at_rest_with_neutral_controls() {
        let state = SkiffState::at_rest();
        let next = state.step(0.0, 0.0);
        assert_eq!(next.pos, [0.0, 0.0]);
        assert_eq!(next.speed, 0.0);
        assert_eq!(next.heading, 0.0);
    }

    #[test]
    fn full_throttle_increases_speed() {
        let state = SkiffState::at_rest();
        let next = state.step(1.0, 0.0);
        assert!(next.speed > 0.0);
    }

    #[test]
    fn full_steering_changes_heading() {
        let state = SkiffState::at_rest();
        let next = state.step(0.0, 1.0);
        assert!(next.heading > 0.0);
    }

    #[test]
    fn drag_decays_speed_with_zero_throttle() {
        let state = SkiffState {
            pos: [0.0, 0.0],
            heading: 0.0,
            speed: 10.0,
        };
        let next = state.step(0.0, 0.0);
        assert!(next.speed < 10.0);
    }
}
