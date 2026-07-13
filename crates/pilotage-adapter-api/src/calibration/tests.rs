//! Verification tests for the calibration contract.
#![allow(clippy::expect_used, clippy::panic)]

use super::error::CalibrationError;
use super::identity::ValidityStatus;
use super::{SIM_FPV_CALIBRATION_HASH, content_hash, sim_fpv_calibration, to_canonical};

/// A time inside the published FPV calibration's effective window.
const NOW_IN_WINDOW_NS: u64 = 1_600_000_000_000_000_000;

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
