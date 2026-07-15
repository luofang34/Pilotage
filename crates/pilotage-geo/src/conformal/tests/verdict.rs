//! Ground-truth tests for the conformal verdict: the four-valued state machine,
//! the structural view/availability/identity gates, the dynamic error budget,
//! policy-sourced limits, and the traceable video/overlay identity.
//!
//! Shares the parent module's fixtures via `use super::*`.

use super::*;

use crate::identity::IntegrityLevel;
use crate::view::{CalibrationId, CalibrationRef, Projection, ProjectionView};
use crate::{AvailabilityReason, ConformalPolicyId, ConformalReason, ConformalState, ViewGeometry};

/// Assess with an overridden view and availability inputs (for the structural
/// gates). A generous policy keeps the sim calibration's static bound within the
/// valid budget, so the derived availability — not the error budget — decides.
fn run_full(bracket: &Bracket, v: &ProjectionView, avail: AvailabilityInputs) -> ConformalFix {
    assess_conformal(bracket, capture(1_500), v, &geom(), avail, &generous(), &[])
}

/// Assess with an overridden resolved geometry (for the calibration-binding and
/// geometry-validation gates).
fn run_geom(bracket: &Bracket, g: &ViewGeometry) -> ConformalFix {
    assess_conformal(
        bracket,
        capture(1_500),
        &view(),
        g,
        availability(),
        &sim(),
        &[],
    )
}

/// Assess the nominal view/camera/scene with a supplied set of runway/path cues.
fn run_path(
    bracket: &Bracket,
    cap: CaptureContext,
    policy: &ConformalPolicy,
    path: &[[f64; 3]],
) -> ConformalFix {
    assess_conformal(bracket, cap, &view(), &geom(), availability(), policy, path)
}

#[test]
fn nominal_low_dynamics_is_valid_with_a_bounded_budget() {
    // A genuinely verified sim calibration + low dynamics reaches Valid under a
    // policy whose valid budget covers the sim calibration's static bound.
    let policy = generous();
    let fix = run(
        &steady(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3]),
        capture(1_500),
        &policy,
    );
    assert_eq!(fix.state, ConformalState::Valid);
    let err = fix.error.expect("interpolation ran");
    assert!(err.total_rad.is_finite() && err.total_rad < policy.valid_error_rad());
    // Every contributor is present and finite (named, sourced terms); the
    // calibration term is the verified artifact's published bound.
    assert!(err.calibration_rad > 0.0 && err.latency_rad > 0.0 && err.clock_rad > 0.0);
}

#[test]
fn clock_domain_mismatch_is_non_conformal_and_removes_cues() {
    let mut cap = capture(1_500);
    cap.capture_epoch = Epoch {
        clock: ClockDomain::VehicleBoot,
        scale: TimeScale::Monotonic,
        nanos: 1_500,
    };
    let fix = run(
        &steady(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3]),
        cap,
        &sim(),
    );
    assert_eq!(
        fix.state,
        ConformalState::NonConformal(ConformalReason::ClockIncoherent)
    );
    assert!(
        fix.cues.is_none(),
        "a mismatched clock leaves no stale alignment"
    );
}

#[test]
fn reordered_bracket_is_a_stream_discontinuity() {
    // newer stamped before older: the timeline is not monotonic.
    let b = Bracket {
        older: sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 3_000, 2),
        newer: sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 1_000, 1),
    };
    let fix = run(&b, capture(2_000), &sim());
    assert_eq!(
        fix.state,
        ConformalState::NonConformal(ConformalReason::StreamDiscontinuity)
    );
    assert!(fix.cues.is_none());
}

#[test]
fn dropped_frames_extrapolate_past_the_policy_limit() {
    // Low dynamics so the budget stays small: the extrapolation *limit* is what
    // trips, proving it comes from the policy, not the error budget.
    let b = Bracket {
        older: sample(Quat::IDENTITY, [1.0, 0.0, 0.0], [0.0; 3], 1_000_000, 1),
        newer: sample(Quat::IDENTITY, [1.0, 0.0, 0.0], [0.0; 3], 2_000_000, 2),
    };
    // Capture 60 ms past the newest sample; simulator limit is 50 ms.
    let past = run(&b, capture(62_000_000), &sim());
    assert_eq!(
        past.state,
        ConformalState::NonConformal(ConformalReason::ExcessiveExtrapolation)
    );
    assert!(past.cues.is_none());
    // 40 ms past is within both the limit and the budget: drawable.
    let within = run(&b, capture(42_000_000), &sim());
    assert!(
        within.state.draws_cues(),
        "40 ms extrapolation is inside 50 ms"
    );
}

#[test]
fn extrapolation_limit_is_sourced_from_the_policy() {
    let b = Bracket {
        older: sample(Quat::IDENTITY, [1.0, 0.0, 0.0], [0.0; 3], 1_000_000, 1),
        newer: sample(Quat::IDENTITY, [1.0, 0.0, 0.0], [0.0; 3], 2_000_000, 2),
    };
    // A tighter policy (30 ms) flips a 40 ms extrapolation from drawable to
    // non-conformal — the same inputs, only the policy differs.
    let tight = ConformalPolicy::new(
        ConformalPolicyId(9),
        1,
        30_000_000,
        10_000_000,
        1.0,
        0.010,
        0.030,
        50.0,
    )
    .expect("valid policy");
    let fix = run(&b, capture(42_000_000), &tight);
    assert_eq!(
        fix.state,
        ConformalState::NonConformal(ConformalReason::ExcessiveExtrapolation)
    );
}

#[test]
fn excessive_angular_rate_suppresses_conformal_cues() {
    let b = steady(Quat::IDENTITY, [50.0, 0.0, 0.0], [2.0, 0.0, 0.0]);
    let fix = run(&b, capture(1_500), &sim());
    assert_eq!(
        fix.state,
        ConformalState::NonConformal(ConformalReason::ExcessiveRate)
    );
    assert!(fix.cues.is_none());
}

#[test]
fn exceeded_error_budget_removes_the_cues() {
    // A large clock-mapping error at speed blows the angular budget.
    let mut cap = capture(1_500);
    cap.clock_error_ns = 100_000_000; // 100 ms
    let fix = run(
        &steady(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3]),
        cap,
        &sim(),
    );
    assert_eq!(
        fix.state,
        ConformalState::NonConformal(ConformalReason::ErrorBudgetExceeded)
    );
    assert!(
        fix.cues.is_none(),
        "exceeded budget must not leave stale alignment"
    );
    assert!(fix.error.expect("ran").total_rad > sim().limited_error_rad());
}

#[test]
fn snapshot_incoherent_pose_is_refused() {
    let mut s = sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 1_000, 1);
    // Break coherence: give the position a different snapshot id than attitude.
    s.position.stamp.snapshot.id = 999;
    let b = Bracket {
        older: s,
        newer: sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 2_000, 2),
    };
    let fix = run(&b, capture(1_500), &sim());
    assert_eq!(
        fix.state,
        ConformalState::NonConformal(ConformalReason::SnapshotIncoherent)
    );
}

#[test]
fn excessive_skew_between_attitude_and_position_is_refused() {
    let mut s = sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 1_000_000, 1);
    // Same snapshot (still coherent) but sampled 20 ms apart — over the 10 ms
    // policy skew limit.
    s.attitude.stamp.acquired_at = epoch(1_000_000);
    s.position.stamp.acquired_at = epoch(21_000_000);
    let newer = sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 30_000_000, 2);
    let fix = run(&Bracket { older: s, newer }, capture(20_000_000), &sim());
    assert_eq!(
        fix.state,
        ConformalState::NonConformal(ConformalReason::ExcessiveSkew)
    );
}

#[test]
fn unreferenced_calibration_is_unavailable() {
    let mut v = view();
    v.calibration.calibration_id = CalibrationId::NONE;
    let fix = run_full(
        &steady(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3]),
        &v,
        availability(),
    );
    assert_eq!(
        fix.state,
        ConformalState::Unavailable(ConformalReason::ViewInvalid)
    );
    assert!(fix.cues.is_none() && fix.error.is_none());
}

#[test]
fn orthographic_view_is_not_a_conformal_view() {
    let mut v = view();
    v.projection = Projection::Orthographic {
        extent_x_m: 100.0,
        extent_y_m: 75.0,
    };
    let fix = run_full(
        &steady(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3]),
        &v,
        availability(),
    );
    assert_eq!(
        fix.state,
        ConformalState::Unavailable(ConformalReason::ViewInvalid)
    );
}

#[test]
fn unavailable_scene_derived_from_a_sample_delegates_the_availability_reason() {
    // The availability is DERIVED from the samples, not asserted: an untrusted
    // position integrity in an endpoint makes the derived scene Unavailable, and
    // the conformal verdict delegates that reason.
    let mut older = sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 1_000, 1);
    older.position.stamp.integrity = IntegrityLevel::Untrusted;
    let b = Bracket {
        older,
        newer: sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 2_000, 2),
    };
    let fix = run_full(&b, &view(), availability());
    assert_eq!(
        fix.state,
        ConformalState::Unavailable(ConformalReason::Availability(AvailabilityReason::Position))
    );
    assert!(fix.cues.is_none());
}

#[test]
fn degraded_scene_marks_the_cues_limited() {
    // A producer-stated external input degrades: the derived scene is Degraded and
    // the cues are drawn but marked reduced.
    let avail = AvailabilityInputs {
        external: ExternalHealth {
            database: InputHealth::Degraded,
            ..healthy_external()
        },
        profile: AvailabilityProfile::simulator(),
    };
    let fix = run_full(
        &steady(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3]),
        &view(),
        avail,
    );
    assert_eq!(
        fix.state,
        ConformalState::Limited(ConformalReason::Availability(AvailabilityReason::Database))
    );
    assert!(
        fix.cues.expect("limited still draws").limited,
        "cues marked reduced"
    );
}

#[test]
fn fix_identity_retains_both_bracket_endpoints_and_the_capture_time() {
    let older = sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 1_000, 11);
    let newer = sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 2_000, 77);
    let b = Bracket { older, newer };
    let cap = capture(1_800);
    let fix = run(&b, cap, &sim());
    // The interpolation reads BOTH endpoints, so both snapshot identities are
    // retained — an interpolated fix belongs to no single snapshot. Video identity
    // is the capture epoch; overlay identity is the pair of coherent snapshots.
    assert_eq!(fix.identity.older_snapshot, older.attitude.stamp.snapshot);
    assert_eq!(fix.identity.newer_snapshot, newer.attitude.stamp.snapshot);
    assert_ne!(
        fix.identity.older_snapshot, fix.identity.newer_snapshot,
        "the two endpoints are distinct snapshots, both kept"
    );
    assert_eq!(
        fix.identity.source_incarnation,
        newer.attitude.stamp.incarnation
    );
    assert_eq!(fix.identity.capture_epoch, cap.capture_epoch);
}

#[test]
fn policy_rejects_non_monotonic_error_bounds() {
    // valid > limited: non-monotonic.
    let bad = ConformalPolicy::new(
        ConformalPolicyId(2),
        1,
        50_000_000,
        10_000_000,
        1.0,
        0.030,
        0.010,
        50.0,
    );
    assert!(matches!(
        bad,
        Err(super::super::ConformalError::InvalidPolicy {
            field: "error_bound"
        })
    ));
}

#[test]
fn policy_rejects_zero_and_non_finite_limits() {
    let zero = ConformalPolicy::new(
        ConformalPolicyId(3),
        1,
        0,
        10_000_000,
        1.0,
        0.010,
        0.030,
        50.0,
    );
    assert!(matches!(
        zero,
        Err(super::super::ConformalError::InvalidPolicy {
            field: "max_extrapolation_ns"
        })
    ));
    let nan_range = ConformalPolicy::new(
        ConformalPolicyId(4),
        1,
        50_000_000,
        10_000_000,
        1.0,
        0.010,
        0.030,
        f64::NAN,
    );
    assert!(matches!(
        nan_range,
        Err(super::super::ConformalError::InvalidPolicy {
            field: "reference_range_m"
        })
    ));
}

#[test]
fn path_cues_ride_the_verdict_and_are_removed_when_budget_exceeded() {
    let path = [[300.0, 0.0, 50.0], [400.0, 20.0, 60.0]];
    // Drawable: the runway/path cues are projected and counted alongside the
    // horizon and flight-path marker.
    let ok = run_path(
        &steady(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3]),
        capture(1_500),
        &sim(),
        &path,
    );
    let cues = ok.cues.expect("valid draws cues");
    assert_eq!(cues.path_count, 2);
    assert!(cues.path[0].is_some(), "the first runway point projects");
    // Exceeded budget removes every cue, including the runway/path cues.
    let mut cap = capture(1_500);
    cap.clock_error_ns = 100_000_000;
    let gone = run_path(
        &steady(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3]),
        cap,
        &sim(),
        &path,
    );
    assert_eq!(
        gone.state,
        ConformalState::NonConformal(ConformalReason::ErrorBudgetExceeded)
    );
    assert!(
        gone.cues.is_none(),
        "exceeded budget removes runway/path cues too"
    );
}

#[test]
fn geometry_from_a_mismatched_calibration_is_not_valid() {
    let bracket = steady(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3]);
    // The geometry is derived from the genuine verified model, but the projection
    // view references a DIFFERENT calibration id — the projection would run through
    // the wrong camera model, so it is refused fail-closed.
    let mut wrong_id = view();
    wrong_id.calibration = CalibrationRef {
        calibration_id: CalibrationId(0x9999_0000),
        content_hash: calibration().content_hash,
    };
    let fix = run_full(&bracket, &wrong_id, availability());
    assert_ne!(fix.state, ConformalState::Valid);
    assert_eq!(
        fix.state,
        ConformalState::Unavailable(ConformalReason::CalibrationMismatch)
    );
    assert!(fix.cues.is_none());

    // Same id but a different content hash is also a mismatch: the artifact behind
    // the id changed.
    let mut wrong_hash = view();
    wrong_hash.calibration.content_hash = [9; 32];
    assert_eq!(
        run_full(&bracket, &wrong_hash, availability()).state,
        ConformalState::Unavailable(ConformalReason::CalibrationMismatch)
    );
}

#[test]
fn assess_conformal_rejects_an_unvalidated_geometry() {
    // Defense in depth: even if a malformed geometry is fabricated in-crate
    // (external callers cannot — the fields are private and derive validates),
    // assess_conformal re-validates and refuses it.
    let mut g = geom();
    g.half_fov_x_tan = f64::NAN;
    assert_eq!(
        run_geom(&steady(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3]), &g).state,
        ConformalState::Unavailable(ConformalReason::ViewInvalid)
    );
}
