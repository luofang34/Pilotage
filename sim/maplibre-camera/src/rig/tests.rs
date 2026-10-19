#![allow(clippy::expect_used, clippy::panic)]

use super::{CameraSpec, FPV_CAMERA, GIMBAL_CAMERA};

#[test]
fn a_detent_frustum_is_the_pinhole_image_of_its_field_of_view() {
    let gimbal = CameraSpec::gimbal((640, 480));
    for detent in 0..gimbal.detents_hfov_rad.len() {
        let frustum = gimbal.frustum(detent);
        let hfov = 2.0 * frustum.left.atan();
        assert!((hfov - gimbal.detents_hfov_rad[detent]).abs() < 1e-12);
        assert!((frustum.left - frustum.right).abs() < 1e-12);
        // Square pixels: the vertical tangent is the horizontal one scaled
        // by the image's height over its width.
        assert!((frustum.top - frustum.left * 480.0 / 640.0).abs() < 1e-12);
        let lens = gimbal.intrinsics(detent);
        assert_eq!((lens.principal_x_px, lens.principal_y_px), (320.0, 240.0));
    }
    let narrowing: Vec<f64> = (0..4)
        .map(|detent| gimbal.intrinsics(detent).focal_px)
        .collect();
    assert!(
        narrowing.windows(2).all(|pair| pair[0] < pair[1]),
        "{narrowing:?}"
    );
}

#[test]
fn every_detent_has_its_own_nonzero_calibration_id() {
    let fpv = CameraSpec::fpv((640, 480));
    let gimbal = CameraSpec::gimbal((640, 480));
    assert_eq!(
        (fpv.camera_id, gimbal.camera_id),
        (FPV_CAMERA, GIMBAL_CAMERA)
    );
    let mut ids: Vec<u32> = (0..4).map(|detent| gimbal.calibration_id(detent)).collect();
    ids.push(fpv.calibration_id(0));
    assert!(ids.iter().all(|id| *id != 0));
    let count = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), count, "ids collide");
    assert_eq!(gimbal.calibration_id(1), 0x4D4C_0202);
}

#[test]
fn a_detent_past_the_last_holds_at_the_last() {
    let gimbal = CameraSpec::gimbal((640, 480));
    assert_eq!(gimbal.detent(0), 0);
    assert_eq!(gimbal.detent(3), 3);
    assert_eq!(gimbal.detent(9), 3);
    assert_eq!(CameraSpec::fpv((640, 480)).detent(2), 0);
}
