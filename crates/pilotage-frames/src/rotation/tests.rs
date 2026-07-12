#![allow(clippy::expect_used, clippy::panic)]

use super::Quat;
use core::f32::consts::FRAC_PI_2;

fn assert_close(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-4, "{a} !~ {b}");
}

fn from_axis_angle(axis: [f32; 3], angle: f32) -> Quat {
    let h = angle / 2.0;
    let s = libm::sinf(h);
    Quat {
        w: libm::cosf(h),
        x: axis[0] * s,
        y: axis[1] * s,
        z: axis[2] * s,
    }
}

#[test]
fn identity_is_level_north() {
    let (r, p, y) = Quat::IDENTITY.to_euler();
    assert_close(r, 0.0);
    assert_close(p, 0.0);
    assert_close(y, 0.0);
}

#[test]
fn pure_roll_extracts_as_roll() {
    let (r, p, y) = from_axis_angle([1.0, 0.0, 0.0], 0.5).to_euler();
    assert_close(r, 0.5);
    assert_close(p, 0.0);
    assert_close(y, 0.0);
}

#[test]
fn pure_pitch_extracts_as_pitch() {
    let (r, p, y) = from_axis_angle([0.0, 1.0, 0.0], -0.3).to_euler();
    assert_close(r, 0.0);
    assert_close(p, -0.3);
    assert_close(y, 0.0);
}

#[test]
fn pure_yaw_extracts_as_yaw() {
    let (r, p, y) = from_axis_angle([0.0, 0.0, 1.0], 1.2).to_euler();
    assert_close(r, 0.0);
    assert_close(p, 0.0);
    assert_close(y, 1.2);
}

#[test]
fn gimbal_edge_is_clamped_not_nan() {
    // Pitch exactly +90°: sinp can drift past 1.0 through float error.
    let q = from_axis_angle([0.0, 1.0, 0.0], FRAC_PI_2);
    let (_, p, _) = q.to_euler();
    assert!(p.is_finite());
    // asin's slope is vertical at the edge, so f32 precision is coarse
    // here; near-90° is all the display needs.
    assert!((p - FRAC_PI_2).abs() < 1e-2, "{p} not near 90 deg");
}
