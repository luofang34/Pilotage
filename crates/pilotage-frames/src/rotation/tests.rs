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

#[test]
fn from_euler_round_trips_through_to_euler() {
    // The convention's owner carries both directions; the round trip
    // pins the contract a downstream copy would otherwise re-derive by
    // hand.
    let cases = [
        (0.0f32, 0.0, 0.0),
        (0.3, -0.2, 1.1),
        (-1.2, 0.7, -2.9),
        (0.05, 1.4, 3.1),
        (2.9, -1.4, -0.01),
    ];
    for (roll, pitch, yaw) in cases {
        let (r, p, y) = Quat::from_euler(roll, pitch, yaw).to_euler();
        assert_close(r, roll);
        assert_close(p, pitch);
        assert_close(y, yaw);
    }
}

#[test]
fn from_euler_is_unit_norm() {
    let q = Quat::from_euler(0.4, -0.9, 2.2);
    let norm = q.w * q.w + q.x * q.x + q.y * q.y + q.z * q.z;
    assert!((norm - 1.0).abs() < 1e-6);
}

#[test]
fn from_euler_matches_axis_angle_singles() {
    // Each single-axis rotation must equal the axis-angle construction
    // the rest of the crate composes from.
    let pairs = [
        (
            Quat::from_euler(0.8, 0.0, 0.0),
            from_axis_angle([1.0, 0.0, 0.0], 0.8),
        ),
        (
            Quat::from_euler(0.0, 0.6, 0.0),
            from_axis_angle([0.0, 1.0, 0.0], 0.6),
        ),
        (
            Quat::from_euler(0.0, 0.0, 1.3),
            from_axis_angle([0.0, 0.0, 1.0], 1.3),
        ),
    ];
    for (a, b) in pairs {
        assert_close(a.w, b.w);
        assert_close(a.x, b.x);
        assert_close(a.y, b.y);
        assert_close(a.z, b.z);
    }
}
