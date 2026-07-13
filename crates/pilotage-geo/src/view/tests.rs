//! Tests for the projection view: the calibration reference, projection
//! payloads, projection boundary, and fail-closed validation.
#![allow(clippy::expect_used, clippy::panic)]

use super::{
    CalibrationId, CalibrationRef, MinificationPolicy, NearFarPolicy, Projection, ProjectionView,
};
use crate::error::GeoError;

fn calibration() -> CalibrationRef {
    CalibrationRef {
        calibration_id: CalibrationId(0x0FED_CBA9),
        content_hash: [7u8; 32],
    }
}

fn view() -> ProjectionView {
    ProjectionView {
        calibration: calibration(),
        projection: Projection::Perspective,
        near_far: NearFarPolicy {
            near_m: 0.1,
            far_m: 100.0,
        },
        minification: MinificationPolicy::Trilinear,
    }
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
fn valid_perspective_view_passes() {
    view()
        .validate()
        .expect("the nominal perspective view is valid");
}

#[test]
fn valid_orthographic_view_passes() {
    let mut v = view();
    v.projection = Projection::Orthographic {
        extent_x_m: 500.0,
        extent_y_m: 375.0,
    };
    v.validate()
        .expect("a well-formed orthographic view is valid");
}

#[test]
fn an_incomplete_calibration_reference_is_refused() {
    let mut v = view();
    v.calibration.calibration_id = CalibrationId::NONE;
    assert!(matches!(
        v.validate(),
        Err(GeoError::IncompleteCalibrationReference)
    ));

    let mut v = view();
    v.calibration.content_hash = [0u8; 32];
    assert!(
        matches!(v.validate(), Err(GeoError::IncompleteCalibrationReference)),
        "an all-zero content hash does not identify an artifact"
    );
}

#[test]
fn orthographic_needs_positive_finite_extents() {
    for (x, y) in [(0.0, 375.0), (500.0, -1.0), (f64::NAN, 375.0)] {
        let mut v = view();
        v.projection = Projection::Orthographic {
            extent_x_m: x,
            extent_y_m: y,
        };
        assert!(
            matches!(
                v.validate(),
                Err(GeoError::InvalidOrthographicExtent { .. })
            ),
            "orthographic extents ({x}, {y}) must be positive finite"
        );
    }
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
fn projection_kind_discriminants_match_the_wire() {
    assert_eq!(Projection::Perspective.kind_u8(), 0);
    assert_eq!(
        Projection::Orthographic {
            extent_x_m: 1.0,
            extent_y_m: 1.0,
        }
        .kind_u8(),
        1
    );
    assert_eq!(
        MinificationPolicy::from_u8(2),
        Some(MinificationPolicy::Trilinear)
    );
    assert_eq!(MinificationPolicy::from_u8(9), None);
}
