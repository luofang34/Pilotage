#![allow(clippy::expect_used, clippy::panic)]

use super::*;
use crate::error::RasterError;

#[test]
fn snaps_zero_and_unit_and_subpixel() {
    assert_eq!(Fx::snap(0.0).expect("finite").raw(), 0);
    assert_eq!(Fx::snap(1.0).expect("finite").raw(), i64::from(ONE));
    // A half pixel snaps to 128 (1/256 units), the subpixel boundary.
    assert_eq!(Fx::snap(0.5).expect("finite").raw(), i64::from(HALF));
    // One quantization step is exactly 1/256 px.
    assert_eq!(Fx::snap(1.0 / 256.0).expect("finite").raw(), 1);
}

#[test]
fn snaps_negative_symmetrically() {
    assert_eq!(Fx::snap(-1.0).expect("finite").raw(), -i64::from(ONE));
    assert_eq!(Fx::snap(-0.5).expect("finite").raw(), -i64::from(HALF));
}

#[test]
fn round_trips_exactly_within_range() {
    for v in [0.0f32, 0.5, -0.5, 123.25, -400.75, 32000.0] {
        assert_eq!(Fx::snap(v).expect("finite").to_f32(), v);
    }
}

#[test]
fn pixel_center_is_half_past_the_index() {
    assert_eq!(Fx::pixel_center(0).to_f32(), 0.5);
    assert_eq!(Fx::pixel_center(10).to_f32(), 10.5);
    assert_eq!(Fx::pixel_center(-1).to_f32(), -0.5);
}

#[test]
fn rejects_out_of_range() {
    assert!(matches!(
        Fx::snap(COORD_LIMIT_PX + 1.0),
        Err(RasterError::CoordinateOutOfRange { .. })
    ));
    assert!(matches!(
        Fx::snap(-(COORD_LIMIT_PX + 1.0)),
        Err(RasterError::CoordinateOutOfRange { .. })
    ));
}

#[test]
fn rejects_non_finite() {
    assert_eq!(Fx::snap(f32::NAN), Err(RasterError::NonFinite));
    assert_eq!(Fx::snap(f32::INFINITY), Err(RasterError::NonFinite));
    assert_eq!(Fx::snap(f32::NEG_INFINITY), Err(RasterError::NonFinite));
}
