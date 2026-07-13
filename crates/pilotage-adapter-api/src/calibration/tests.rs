//! Verification tests for the calibration contract.
#![allow(clippy::expect_used, clippy::panic)]

use pilotage_frames::FrameId;

use super::CameraCalibration;
use super::error::CalibrationError;
use super::identity::ValidityStatus;
use super::{SIM_FPV_CALIBRATION_HASH, content_hash, sim_fpv_calibration, to_canonical};

/// A time inside the published FPV calibration's effective window.
const NOW_IN_WINDOW_NS: u64 = 1_600_000_000_000_000_000;

/// Re-records the (now consistent) hash of a mutated calibration and verifies
/// it, so a fault-injection test proves the bytes and their recorded hash AGREE
/// yet the semantically invalid geometry is still rejected.
fn verify_hash_consistent(cal: &CameraCalibration) -> CalibrationError {
    let recorded = cal.content_hash();
    cal.verify(recorded, NOW_IN_WINDOW_NS)
        .expect_err("a hash-consistent but semantically invalid calibration must be rejected")
}

#[test]
fn sim_fpv_hash_is_recorded_and_verifies() {
    let cal = sim_fpv_calibration();
    assert_eq!(
        cal.content_hash(),
        SIM_FPV_CALIBRATION_HASH,
        "recorded hash must match the canonical content"
    );
    cal.verify(SIM_FPV_CALIBRATION_HASH, NOW_IN_WINDOW_NS)
        .expect("published calibration verifies inside its window");
}

#[test]
fn canonical_form_is_deterministic() {
    let cal = sim_fpv_calibration();
    assert_eq!(to_canonical(&cal), to_canonical(&cal));
    assert_eq!(content_hash(&cal), content_hash(&cal));
}

#[test]
fn mutated_value_fails_the_recorded_hash() {
    let mut cal = sim_fpv_calibration();
    // Alter one intrinsic without re-recording the hash.
    cal.geometry.intrinsics.focal_x_px += 1.0;
    let err = cal
        .verify(SIM_FPV_CALIBRATION_HASH, NOW_IN_WINDOW_NS)
        .expect_err("a mutated value must fail the recorded hash");
    assert!(matches!(err, CalibrationError::ContentHashMismatch { .. }));
}

#[test]
fn revoked_calibration_does_not_verify() {
    let mut cal = sim_fpv_calibration();
    cal.identity.status = ValidityStatus::Revoked;
    let recorded = cal.content_hash();
    let err = cal
        .verify(recorded, NOW_IN_WINDOW_NS)
        .expect_err("a revoked calibration must not verify");
    assert!(matches!(err, CalibrationError::NotValid { .. }));
}

#[test]
fn expired_calibration_does_not_verify() {
    let cal = sim_fpv_calibration();
    let err = cal
        .verify(SIM_FPV_CALIBRATION_HASH, 0)
        .expect_err("a time before the window must not verify");
    assert!(matches!(err, CalibrationError::Expired { .. }));
}

#[test]
fn wrong_camera_is_rejected() {
    let cal = sim_fpv_calibration();
    cal.verify_camera(cal.identity.camera_id)
        .expect("the calibration's own camera matches");
    let err = cal
        .verify_camera(cal.identity.camera_id + 1)
        .expect_err("a different camera must be rejected");
    assert!(matches!(err, CalibrationError::WrongCamera { .. }));
}

// ---- semantic validation: hash-consistent but invalid geometry -------------

#[test]
fn sim_fpv_passes_semantic_validation() {
    sim_fpv_calibration()
        .validate()
        .expect("the published calibration is semantically valid");
}

#[test]
fn non_finite_field_is_rejected() {
    let mut cal = sim_fpv_calibration();
    cal.geometry.intrinsics.focal_x_px = f64::NAN;
    assert!(matches!(
        verify_hash_consistent(&cal),
        CalibrationError::NonFinite { .. }
    ));
}

#[test]
fn zero_viewport_is_rejected() {
    let mut cal = sim_fpv_calibration();
    cal.geometry.viewport.width_px = 0;
    assert!(matches!(
        verify_hash_consistent(&cal),
        CalibrationError::InvalidViewport { .. }
    ));
}

#[test]
fn out_of_range_fov_is_rejected() {
    let mut cal = sim_fpv_calibration();
    cal.geometry.fov.horizontal_rad = 4.0; // > PI
    assert!(matches!(
        verify_hash_consistent(&cal),
        CalibrationError::InvalidFieldOfView { .. }
    ));
}

#[test]
fn non_positive_focal_is_rejected() {
    let mut cal = sim_fpv_calibration();
    cal.geometry.intrinsics.focal_x_px = -1.0;
    assert!(matches!(
        verify_hash_consistent(&cal),
        CalibrationError::NonPositiveFocal { .. }
    ));
}

#[test]
fn principal_point_outside_viewport_is_rejected() {
    let mut cal = sim_fpv_calibration();
    cal.geometry.intrinsics.principal_x_px = 10_000.0;
    assert!(matches!(
        verify_hash_consistent(&cal),
        CalibrationError::PrincipalPointOutOfBounds { .. }
    ));
}

#[test]
fn wrong_extrinsic_frames_are_rejected() {
    let mut cal = sim_fpv_calibration();
    cal.geometry.extrinsics.from_frame = FrameId::Ned;
    assert!(matches!(
        verify_hash_consistent(&cal),
        CalibrationError::FrameMismatch { .. }
    ));
}

#[test]
fn non_unit_quaternion_is_rejected() {
    let mut cal = sim_fpv_calibration();
    cal.geometry.extrinsics.rotation_quat_wxyz = [2.0, 0.0, 0.0, 0.0];
    assert!(matches!(
        verify_hash_consistent(&cal),
        CalibrationError::NonUnitQuaternion { .. }
    ));
}

#[test]
fn non_unit_boresight_is_rejected() {
    let mut cal = sim_fpv_calibration();
    cal.geometry.boresight.direction_camera = [2.0, 0.0, 0.0];
    assert!(matches!(
        verify_hash_consistent(&cal),
        CalibrationError::NonUnitBoresight { .. }
    ));
}

#[test]
fn inverted_effective_period_is_rejected() {
    let mut cal = sim_fpv_calibration();
    cal.identity.effective.end_unix_ns = cal.identity.effective.start_unix_ns;
    assert!(matches!(
        verify_hash_consistent(&cal),
        CalibrationError::InvalidEffectivePeriod { .. }
    ));
}

#[test]
fn negative_residuals_are_rejected() {
    let mut cal = sim_fpv_calibration();
    cal.identity.residuals.rms_px = -1.0;
    assert!(matches!(
        verify_hash_consistent(&cal),
        CalibrationError::InvalidResiduals { .. }
    ));
}

#[test]
fn inconsistent_alignment_budget_is_rejected() {
    let mut cal = sim_fpv_calibration();
    cal.budget.total_angular_bound_rad = 999.0;
    assert!(matches!(
        verify_hash_consistent(&cal),
        CalibrationError::InvalidAlignmentBudget { .. }
    ));
}
