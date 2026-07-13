//! Tests for the projection view: derived FOV, projection boundary, and
//! fail-closed validation.
#![allow(clippy::expect_used, clippy::panic)]

use pilotage_frames::{FrameId, Quat};

use super::{
    CameraPose, MinificationPolicy, NearFarPolicy, OpticalConvention, ProjectionKind,
    ProjectionView, Viewport,
};
use crate::error::GeoError;

fn view() -> ProjectionView {
    ProjectionView {
        viewport: Viewport {
            width_px: 320,
            height_px: 240,
        },
        focal_x_px: 190.0,
        focal_y_px: 190.0,
        projection: ProjectionKind::Perspective,
        near_far: NearFarPolicy {
            near_m: 0.1,
            far_m: 100.0,
        },
        minification: MinificationPolicy::Trilinear,
        convention: OpticalConvention::OpenCv,
        camera: CameraPose {
            translation_m: [1.1, 0.0, 0.3],
            attitude: Quat::IDENTITY,
            from_frame: FrameId::Body,
            to_frame: FrameId::Installation,
        },
    }
}

#[test]
fn field_of_view_is_derived_from_viewport_and_focal() {
    let fov = view().field_of_view();
    let expected_h = 2.0 * libm::atan(160.0 / 190.0);
    let expected_v = 2.0 * libm::atan(120.0 / 190.0);
    assert!((fov.horizontal_rad - expected_h).abs() < 1e-12);
    assert!((fov.vertical_rad - expected_v).abs() < 1e-12);
}

#[test]
fn projection_boundary_is_inclusive_and_deterministic() {
    let policy = view().near_far;
    assert!(policy.contains_depth(0.1), "on the near plane is inside");
    assert!(policy.contains_depth(100.0), "on the far plane is inside");
    assert!(
        !policy.contains_depth(0.099_999),
        "just inside the near plane is out"
    );
    assert!(
        !policy.contains_depth(100.000_001),
        "just beyond the far plane is out"
    );
    assert!(policy.contains_depth(50.0));
}

#[test]
fn valid_view_passes() {
    view().validate().expect("the nominal view is valid");
}

#[test]
fn degenerate_viewport_is_refused() {
    let mut v = view();
    v.viewport.width_px = 0;
    assert!(matches!(
        v.validate(),
        Err(GeoError::InvalidViewport { .. })
    ));
}

#[test]
fn non_positive_focal_is_refused() {
    let mut v = view();
    v.focal_x_px = 0.0;
    assert!(matches!(
        v.validate(),
        Err(GeoError::NonPositiveFocal { .. })
    ));
}

#[test]
fn invalid_near_far_is_refused() {
    let mut v = view();
    v.near_far = NearFarPolicy {
        near_m: 10.0,
        far_m: 5.0,
    };
    assert!(matches!(v.validate(), Err(GeoError::InvalidNearFar { .. })));
    v.near_far = NearFarPolicy {
        near_m: 0.0,
        far_m: 5.0,
    };
    assert!(matches!(v.validate(), Err(GeoError::InvalidNearFar { .. })));
}

#[test]
fn wrong_camera_frames_are_refused() {
    let mut v = view();
    v.camera.from_frame = FrameId::Ned;
    assert!(v.validate().is_err());
}

#[test]
fn view_enums_round_trip_and_reject_unknown() {
    assert_eq!(
        ProjectionKind::from_u8(1),
        Some(ProjectionKind::Orthographic)
    );
    assert_eq!(ProjectionKind::from_u8(9), None);
    assert_eq!(
        MinificationPolicy::from_u8(2),
        Some(MinificationPolicy::Trilinear)
    );
    assert_eq!(MinificationPolicy::from_u8(9), None);
    assert_eq!(
        OpticalConvention::from_u8(0),
        Some(OpticalConvention::OpenCv)
    );
    assert_eq!(OpticalConvention::from_u8(9), None);
}
