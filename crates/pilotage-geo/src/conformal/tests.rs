//! Ground-truth tests for bounded conformal projection (HUD-03): the projection
//! and interpolation half. The verdict/state-machine half is in [`verdict`].
//!
//! Every test is deterministic and synchronizes on values, never on time. The
//! projection is cross-checked against an independent f64 rotation-matrix oracle
//! (a different arithmetic path than the production quaternion sandwich).
#![allow(clippy::expect_used, clippy::panic)]

mod verdict;

use pilotage_frames::{ClockDomain, Epoch, Quat, TimeScale};

use pilotage_camera_calibration::{
    SIM_FPV_CALIBRATION_HASH, SIM_FPV_CALIBRATION_ID, VerifiedCameraModel, sim_fpv_calibration,
};

use super::{
    Bracket, CaptureContext, ConformalFix, ConformalPolicy, KinematicSample, ViewGeometry,
    assess_conformal, down_in_camera, project_path_cue,
};
use crate::SvsAvailability;
use crate::datum::{
    BaroSettingId, DatumRealizationId, GeodeticPosition, GeoidModelId, HorizontalDatum,
    LocalOriginId, TerrainRefId, VerticalDatum, VerticalPosition,
};
use crate::identity::{
    AttitudeQuality, CoherentSnapshot, IntegrityLevel, PositionQuality, SourceIncarnation,
    SourceStamp, StatedAttitude, StatedPosition,
};
use crate::view::{
    CalibrationId, CalibrationRef, MinificationPolicy, NearFarPolicy, Projection, ProjectionView,
};

const DEG: f64 = core::f64::consts::PI / 180.0;

// --- fixtures (shared with the `verdict` submodule via `use super::*`) ------

fn epoch(nanos: u64) -> Epoch {
    Epoch {
        clock: ClockDomain::Simulation,
        scale: TimeScale::Monotonic,
        nanos,
    }
}

fn stamp(nanos: u64, snap_id: u64) -> SourceStamp {
    SourceStamp {
        source_id: 7,
        incarnation: SourceIncarnation([9; 16]),
        generation: 1,
        sequence: (nanos & 0xffff_ffff) as u32,
        acquired_at: epoch(nanos),
        integrity: IntegrityLevel::Trusted,
        snapshot: CoherentSnapshot {
            producer: SourceIncarnation([5; 16]),
            generation: 1,
            id: snap_id,
        },
    }
}

fn geopos() -> GeodeticPosition {
    let vertical = VerticalPosition::new(
        300.0,
        VerticalDatum::Ellipsoid,
        GeoidModelId::UNDECLARED,
        TerrainRefId::UNDECLARED,
        BaroSettingId::UNDECLARED,
        LocalOriginId::UNDECLARED,
    )
    .expect("ellipsoidal height needs no extra identity");
    GeodeticPosition::new(
        37.0,
        -122.0,
        HorizontalDatum::Wgs84,
        DatumRealizationId::UNDECLARED,
        vertical,
    )
    .expect("a WGS-84 position is well-formed")
}

/// A sample whose attitude and position share one coherent snapshot stamp (zero
/// skew), with small, conformal-grade accuracies.
fn sample(att: Quat, vel: [f64; 3], rate: [f32; 3], nanos: u64, snap_id: u64) -> KinematicSample {
    let s = stamp(nanos, snap_id);
    KinematicSample {
        attitude: StatedAttitude {
            attitude: att,
            stamp: s,
            quality: AttitudeQuality { angular_mrad: 2 },
        },
        position: StatedPosition {
            position: geopos(),
            stamp: s,
            quality: PositionQuality {
                horizontal_mm: 100,
                vertical_mm: 100,
            },
        },
        velocity_ned_mps: vel,
        body_rate_rps: rate,
    }
}

/// A steady two-sample bracket 1 ms apart at the same pose/velocity/rate, ids
/// declared and in order.
fn steady(att: Quat, vel: [f64; 3], rate: [f32; 3]) -> Bracket {
    Bracket {
        older: sample(att, vel, rate, 1_000, 1),
        newer: sample(att, vel, rate, 2_000, 2),
    }
}

/// Aerospace ZYX (roll, pitch, yaw) body→NED quaternion, matching the presentation
/// convention used across the workspace.
fn euler_quat(roll_deg: f32, pitch_deg: f32, yaw_deg: f32) -> Quat {
    let d = core::f32::consts::PI / 180.0;
    let (r, p, y) = (roll_deg * d / 2.0, pitch_deg * d / 2.0, yaw_deg * d / 2.0);
    let (cr, sr) = (libm::cosf(r), libm::sinf(r));
    let (cp, sp) = (libm::cosf(p), libm::sinf(p));
    let (cy, sy) = (libm::cosf(y), libm::sinf(y));
    Quat {
        w: cr * cp * cy + sr * sp * sy,
        x: sr * cp * cy - cr * sp * sy,
        y: cr * sp * cy + sr * cp * sy,
        z: cr * cp * sy - sr * sp * cy,
    }
}

/// A time inside the published sim FPV calibration's effective window.
pub(super) const NOW_IN_WINDOW_NS: u64 = 1_600_000_000_000_000_000;

/// The calibration reference the sim model — and therefore the projection view —
/// is bound to (the published sim FPV identity and its content hash).
pub(super) fn calibration() -> CalibrationRef {
    CalibrationRef {
        calibration_id: CalibrationId(SIM_FPV_CALIBRATION_ID),
        content_hash: SIM_FPV_CALIBRATION_HASH,
    }
}

/// A genuinely hash-verified camera model, minted the only way one can be — from
/// the published sim calibration through its verify-and-mint path. The sim camera
/// is forward-looking (body FRD → OpenCV optical).
pub(super) fn verified_model() -> VerifiedCameraModel {
    sim_fpv_calibration()
        .verified_camera_model(SIM_FPV_CALIBRATION_HASH, NOW_IN_WINDOW_NS)
        .expect("the published sim calibration verifies and mints")
}

/// The resolved geometry, obtained the only way it can be — by deriving it from a
/// genuinely verified camera model.
fn geom() -> ViewGeometry {
    ViewGeometry::derive(&verified_model()).expect("the verified model derives geometry")
}

fn view() -> ProjectionView {
    ProjectionView {
        calibration: calibration(),
        projection: Projection::Perspective,
        near_far: NearFarPolicy {
            near_m: 0.5,
            far_m: 5000.0,
        },
        minification: MinificationPolicy::Trilinear,
    }
}

fn capture(nanos: u64) -> CaptureContext {
    CaptureContext {
        capture_epoch: epoch(nanos),
        clock_error_ns: 100_000,
        pipeline_latency_ns: 2_000_000,
    }
}

fn sim() -> ConformalPolicy {
    ConformalPolicy::simulator()
}

/// A policy whose valid/limited budget comfortably covers the published sim
/// calibration's static alignment bound (~0.0117 rad, which itself exceeds the
/// tight simulator valid threshold). Used where a test isolates the "genuine
/// verified calibration → Valid" or the availability-decides path from the
/// error budget.
pub(super) fn generous() -> ConformalPolicy {
    ConformalPolicy::new(
        crate::ConformalPolicyId(7),
        1,
        50_000_000,
        10_000_000,
        1.0,
        0.050,
        0.100,
        50.0,
    )
    .expect("a monotonic policy is valid")
}

/// Assess with the nominal view/camera, an available scene, and no path cues.
fn run(bracket: &Bracket, cap: CaptureContext, policy: &ConformalPolicy) -> ConformalFix {
    assess_conformal(
        bracket,
        cap,
        &view(),
        &geom(),
        SvsAvailability::Available,
        policy,
        &[],
    )
}

fn approx(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

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
