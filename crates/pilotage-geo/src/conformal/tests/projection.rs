//! Projection and interpolation geometry: the horizon line, flight-path marker,
//! and runway/path cues, cross-checked against an independent f64 rotation-matrix
//! oracle (a different arithmetic path than the production quaternion sandwich).
//!
//! Shares the parent module's fixtures via `use super::*`.

use super::super::{down_in_camera, project_path_cue};
use super::*;

#[test]
fn forward_camera_mount_maps_body_axes_to_opencv_axes() {
    let bc = geom().body_to_camera;
    let near = |v: [f64; 3], e: [f64; 3]| (0..3).all(|i| approx(v[i], e[i], 1e-6));
    assert!(
        near(bc.rotate([1.0, 0.0, 0.0]), [0.0, 0.0, 1.0]),
        "fwd -> optical axis"
    );
    assert!(
        near(bc.rotate([0.0, 1.0, 0.0]), [1.0, 0.0, 0.0]),
        "right -> image +x"
    );
    assert!(
        near(bc.rotate([0.0, 0.0, 1.0]), [0.0, 1.0, 0.0]),
        "down -> image +y"
    );
}

// --- projection: horizon ----------------------------------------------------

#[test]
fn level_horizon_is_centered_and_horizontal() {
    let cues = run(
        &steady(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3]),
        capture(1_500),
        &sim(),
    )
    .cues
    .expect("level flight is conformal");
    // 0*x + 1*y + 0 = 0 -> y = 0, horizontal through the image center.
    assert!(approx(cues.horizon.a, 0.0, 1e-6) && approx(cues.horizon.c, 0.0, 1e-6));
    assert!(
        cues.horizon.b.abs() > 0.5,
        "a horizontal line has a nonzero y-coefficient"
    );
}

#[test]
fn nose_up_drops_the_horizon_below_center() {
    let q = euler_quat(0.0, 10.0, 0.0);
    let cues = run(
        &steady(q, [50.0, 0.0, 0.0], [0.0; 3]),
        capture(1_500),
        &sim(),
    )
    .cues
    .expect("10 deg nose-up is conformal");
    // Line a*x + b*y + c = 0 at x=0 gives y = -c/b; nose-up puts the horizon
    // below center (+y is down), at tan(pitch).
    let y_at_center = -cues.horizon.c / cues.horizon.b;
    assert!(
        approx(y_at_center, libm::tan(10.0 * DEG), 1e-4),
        "horizon y {y_at_center} should be tan(10 deg)"
    );
}

#[test]
fn high_pitch_and_bank_stay_finite_and_rotate_the_horizon() {
    let q = euler_quat(30.0, 80.0, 0.0);
    let cues = run(
        &steady(q, [40.0, 0.0, 10.0], [0.0; 3]),
        capture(1_500),
        &sim(),
    )
    .cues
    .expect("high pitch/bank is still conformal at zero rate");
    let h = cues.horizon;
    assert!(h.a.is_finite() && h.b.is_finite() && h.c.is_finite());
    assert!(
        h.a.abs() > 1e-3,
        "bank rotates the horizon (nonzero x-coefficient)"
    );
}

#[test]
fn straight_ahead_flight_path_marker_sits_on_boresight() {
    let fpm = run(
        &steady(Quat::IDENTITY, [60.0, 0.0, 0.0], [0.0; 3]),
        capture(1_500),
        &sim(),
    )
    .cues
    .expect("conformal")
    .flight_path
    .expect("a forward velocity has a drawable marker");
    assert!(approx(fpm.x, 0.0, 1e-6) && approx(fpm.y, 0.0, 1e-6));
    assert!(fpm.within_fov);
}

#[test]
fn crosswind_offsets_the_flight_path_marker_sideways() {
    // Heading north, level, but drifting east: the marker shifts right by the
    // drift angle, off the nose.
    let fpm = run(
        &steady(Quat::IDENTITY, [50.0, 10.0, 0.0], [0.0; 3]),
        capture(1_500),
        &sim(),
    )
    .cues
    .expect("conformal")
    .flight_path
    .expect("drawable");
    assert!(
        approx(fpm.x, 10.0 / 50.0, 1e-6),
        "x = east/north drift tangent"
    );
    assert!(approx(fpm.y, 0.0, 1e-6), "no vertical drift");
    assert!(fpm.within_fov);
}

#[test]
fn flight_path_marker_off_scale_past_the_field_boundary() {
    let inside = run(
        &steady(Quat::IDENTITY, [50.0, 40.0, 0.0], [0.0; 3]),
        capture(1_500),
        &sim(),
    )
    .cues
    .expect("conformal")
    .flight_path
    .expect("drawable");
    assert!(inside.within_fov, "tan ~0.8 is inside the 40 deg half-FOV");

    let outside = run(
        &steady(Quat::IDENTITY, [50.0, 60.0, 0.0], [0.0; 3]),
        capture(1_500),
        &sim(),
    )
    .cues
    .expect("conformal")
    .flight_path
    .expect("drawable");
    assert!(!outside.within_fov, "tan 1.2 is the off-scale indication");
}

#[test]
fn runway_path_cue_projects_clips_and_flags_off_scale() {
    let g = geom();
    let nf = view().near_far; // near 0.5 m, far 5000 m
    let att = Quat::IDENTITY; // level, forward-looking camera

    // A runway point 300 m ahead and 50 m below the ownship projects below center
    // (positive y is down) and on-scale.
    let ahead = project_path_cue(att, [300.0, 0.0, 50.0], nf, &g).expect("in front, in range");
    assert!(approx(ahead.x, 0.0, 1e-9) && approx(ahead.y, 50.0 / 300.0, 1e-9));
    assert!(ahead.within_fov);

    // A point in front but far to the right is off-scale (marked, not dropped).
    let side = project_path_cue(att, [100.0, 200.0, 0.0], nf, &g).expect("in front");
    assert!(!side.within_fov, "tan 2.0 is beyond the 40 deg half-FOV");

    // Behind the camera: removed, never drawn behind the eye.
    assert!(
        project_path_cue(att, [-300.0, 0.0, 50.0], nf, &g).is_none(),
        "a point behind the camera is removed"
    );
    // Beyond the far clip: removed by the projection view's near/far policy.
    assert!(
        project_path_cue(att, [6000.0, 0.0, 0.0], nf, &g).is_none(),
        "a point past the far plane is clipped"
    );
    // Inside the near clip: also removed.
    assert!(
        project_path_cue(att, [0.2, 0.0, 0.0], nf, &g).is_none(),
        "a point inside the near plane is clipped"
    );
}

// --- independent f64 oracle -------------------------------------------------

/// Rotation matrix from a quaternion — the standard closed form, an arithmetic
/// path independent of the production `Quat::rotate` cross-product sandwich.
fn rot(q: Quat) -> [[f64; 3]; 3] {
    let (w, x, y, z) = (
        f64::from(q.w),
        f64::from(q.x),
        f64::from(q.y),
        f64::from(q.z),
    );
    [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - w * z),
            2.0 * (x * z + w * y),
        ],
        [
            2.0 * (x * y + w * z),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - w * x),
        ],
        [
            2.0 * (x * z - w * y),
            2.0 * (y * z + w * x),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ]
}

fn matvec(m: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    let row = |r: [f64; 3]| r[0] * v[0] + r[1] * v[1] + r[2] * v[2];
    [row(m[0]), row(m[1]), row(m[2])]
}

fn transpose(m: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    [
        [m[0][0], m[1][0], m[2][0]],
        [m[0][1], m[1][1], m[2][1]],
        [m[0][2], m[1][2], m[2][2]],
    ]
}

#[test]
fn horizon_matches_the_independent_f64_reference() {
    let bc = geom().body_to_camera;
    for &(r, p, yaw) in &[
        (0.0, 0.0, 0.0),
        (0.0, 10.0, 0.0),
        (30.0, 80.0, 0.0),
        (-45.0, -20.0, 33.0),
        (170.0, 5.0, -95.0),
    ] {
        let att = euler_quat(r, p, yaw);
        let got = down_in_camera(att, bc);
        // Oracle: NED down into body via R(att)^T, then body into camera via R(bc).
        let down_body = matvec(transpose(rot(att)), [0.0, 0.0, 1.0]);
        let want = matvec(rot(bc), down_body);
        for i in 0..3 {
            assert!(
                approx(got[i], want[i], 1e-6),
                "comp {i}: got {} want {} at r{r} p{p} y{yaw}",
                got[i],
                want[i]
            );
        }
    }
}

#[test]
fn static_pose_interpolates_to_the_same_attitude() {
    let b = steady(euler_quat(5.0, -8.0, 42.0), [50.0, 0.0, 0.0], [0.0; 3]);
    let a = run(&b, capture(1_000), &sim()).cues.expect("valid").horizon;
    let c = run(&b, capture(1_900), &sim()).cues.expect("valid").horizon;
    // A static pose yields the same horizon regardless of capture time (nlerp of
    // two equal quaternions is that quaternion, up to renormalization rounding).
    assert!(approx(a.a, c.a, 1e-6) && approx(a.b, c.b, 1e-6) && approx(a.c, c.c, 1e-6));
}

#[test]
fn constant_rate_interpolates_the_midpoint_attitude() {
    let b = Bracket {
        older: sample(
            euler_quat(0.0, 0.0, 0.0),
            [50.0, 0.0, 0.0],
            [0.0, 0.0, 0.17],
            1_000_000,
            1,
        ),
        newer: sample(
            euler_quat(0.0, 0.0, 20.0),
            [50.0, 0.0, 0.0],
            [0.0, 0.0, 0.17],
            3_000_000,
            2,
        ),
    };
    let fix = run(&b, capture(2_000_000), &sim());
    // At the midpoint the interpolated attitude is ~10 deg yaw. A north-pointing
    // velocity then projects to a marker at x = -tan(yaw): recover the yaw from it.
    let fpm = fix.cues.expect("conformal").flight_path.expect("drawable");
    assert!(
        approx(fpm.x, -libm::tan(10.0 * DEG), 2e-3),
        "midpoint yaw ~10 deg puts a north-velocity marker at -tan(10 deg), got x={}",
        fpm.x
    );
}
