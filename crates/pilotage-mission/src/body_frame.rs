//! Horizontal NED to body-FRD rotation and yaw geometry, pure `f64`.

use core::f64::consts::PI;

/// Rotates a horizontal NED velocity into body FRD at yaw `psi`
/// (radians, zero north, positive east):
/// `vx = vn·cosψ + ve·sinψ`, `vy = −vn·sinψ + ve·cosψ`.
pub(crate) fn ned_to_body(vn: f64, ve: f64, psi: f64) -> (f64, f64) {
    let (sin, cos) = psi.sin_cos();
    (vn * cos + ve * sin, -vn * sin + ve * cos)
}

/// Bearing of a horizontal NED vector, radians in `(-π, π]`, zero
/// north, positive toward east.
pub(crate) fn bearing_rad(vn: f64, ve: f64) -> f64 {
    ve.atan2(vn)
}

/// Wraps an angle to `(-π, π]` so a heading error never asks for the
/// long way around.
pub(crate) fn wrap_to_pi(angle: f64) -> f64 {
    let wrapped = (-angle + PI).rem_euclid(2.0 * PI);
    -(wrapped - PI)
}

/// Clamps to `[-limit, limit]`.
pub(crate) fn clamp_symmetric(value: f64, limit: f64) -> f64 {
    value.clamp(-limit, limit)
}

/// Caps a horizontal vector's magnitude by scaling, so the ceiling
/// trades speed for nothing else — the direction survives.
pub(crate) fn cap_horizontal(vn: f64, ve: f64, max: f64) -> (f64, f64) {
    let speed = (vn * vn + ve * ve).sqrt();
    if speed <= max || speed <= 0.0 {
        (vn, ve)
    } else {
        let scale = max / speed;
        (vn * scale, ve * scale)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use core::f64::consts::{FRAC_PI_2, PI};

    use super::{bearing_rad, cap_horizontal, ned_to_body, wrap_to_pi};

    const EPS: f64 = 1e-12;

    #[test]
    fn rotation_signs_pinned_by_hand_computed_cases() {
        // Facing north: body equals NED.
        let (vx, vy) = ned_to_body(1.0, 0.5, 0.0);
        assert!((vx - 1.0).abs() < EPS && (vy - 0.5).abs() < EPS);
        // Facing east, moving north: pure leftward body motion.
        let (vx, vy) = ned_to_body(1.0, 0.0, FRAC_PI_2);
        assert!(vx.abs() < EPS && (vy + 1.0).abs() < EPS);
        // Facing east, moving east: pure forward.
        let (vx, vy) = ned_to_body(0.0, 1.0, FRAC_PI_2);
        assert!((vx - 1.0).abs() < EPS && vy.abs() < EPS);
        // Facing northeast, moving north: forward-left at 45 degrees.
        let (vx, vy) = ned_to_body(1.0, 0.0, PI / 4.0);
        let expected = FRAC_PI_2.sin() / 2.0_f64.sqrt();
        assert!((vx - expected).abs() < 1e-9 && (vy + expected).abs() < 1e-9);
    }

    #[test]
    fn bearing_is_zero_north_positive_east() {
        assert!(bearing_rad(1.0, 0.0).abs() < EPS);
        assert!((bearing_rad(0.0, 1.0) - FRAC_PI_2).abs() < EPS);
        assert!((bearing_rad(-1.0, 0.0) - PI).abs() < EPS);
        assert!((bearing_rad(0.0, -1.0) + FRAC_PI_2).abs() < EPS);
    }

    #[test]
    fn wrap_keeps_errors_on_the_short_way() {
        assert!((wrap_to_pi(3.0 * PI / 2.0) + FRAC_PI_2).abs() < EPS);
        assert!((wrap_to_pi(-3.0 * PI / 2.0) - FRAC_PI_2).abs() < EPS);
        assert!((wrap_to_pi(PI) - PI).abs() < EPS);
        assert!(wrap_to_pi(2.0 * PI).abs() < EPS);
    }

    #[test]
    fn horizontal_cap_scales_without_rotating() {
        let (vn, ve) = cap_horizontal(3.0, 4.0, 2.5);
        let speed = (vn * vn + ve * ve).sqrt();
        assert!((speed - 2.5).abs() < 1e-9);
        assert!((vn / ve - 3.0 / 4.0).abs() < 1e-9);
        let (vn, ve) = cap_horizontal(1.0, 1.0, 2.5);
        assert!((vn - 1.0).abs() < EPS && (ve - 1.0).abs() < EPS);
    }
}
