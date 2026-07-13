//! Round-trip and fail-closed decode tests for the SVS frame ABI.
#![allow(clippy::expect_used, clippy::panic)]

use pilotage_frames::{ClockDomain, Epoch, FrameId, Quat, TimeScale};

use super::{ABI_VERSION, SVS_FRAME_LEN, SvsFrame, decode_frame, encode_frame};
use crate::availability::{AvailabilityReason, SvsAvailability};
use crate::datum::{
    GeodeticPosition, GeoidModelId, HorizontalDatum, LocalOriginId, VerticalDatum, VerticalPosition,
};
use crate::error::AbiError;
use crate::identity::{
    Accuracy, IntegrityLevel, SnapshotId, SourceIncarnation, SourceStamp, StatedAttitude,
    StatedPosition,
};
use crate::view::{
    CameraPose, MinificationPolicy, NearFarPolicy, OpticalConvention, ProjectionKind,
    ProjectionView, Viewport,
};

fn stamp() -> SourceStamp {
    SourceStamp {
        source_id: 42,
        incarnation: SourceIncarnation([3; 16]),
        generation: 1,
        sequence: 7,
        acquired_at: Epoch {
            clock: ClockDomain::Gnss,
            scale: TimeScale::Gps,
            nanos: 1_700_000_000_000_000_000,
        },
        integrity: IntegrityLevel::Trusted,
        accuracy: Accuracy {
            horizontal_mm: 1500,
            vertical_mm: 3000,
        },
        snapshot: SnapshotId(99),
    }
}

fn frame() -> SvsFrame {
    let vertical = VerticalPosition::new(
        250.0,
        VerticalDatum::Msl,
        GeoidModelId(1),
        LocalOriginId::UNDECLARED,
    )
    .expect("well-formed vertical");
    let position = GeodeticPosition::new(37.62, -122.38, HorizontalDatum::Wgs84, vertical)
        .expect("well-formed position");
    SvsFrame {
        position: StatedPosition {
            position,
            stamp: stamp(),
        },
        attitude: StatedAttitude {
            attitude: Quat::IDENTITY,
            stamp: stamp(),
        },
        view: ProjectionView {
            viewport: Viewport {
                width_px: 320,
                height_px: 240,
            },
            focal_x_px: 190.0,
            focal_y_px: 190.0,
            projection: ProjectionKind::Perspective,
            near_far: NearFarPolicy {
                near_m: 0.1,
                far_m: 20_000.0,
            },
            minification: MinificationPolicy::Trilinear,
            convention: OpticalConvention::OpenCv,
            camera: CameraPose {
                translation_m: [1.1, 0.0, 0.3],
                attitude: Quat::IDENTITY,
                from_frame: FrameId::Body,
                to_frame: FrameId::Installation,
            },
        },
        availability: SvsAvailability::Degraded(AvailabilityReason::Coverage),
    }
}

#[test]
fn frame_round_trips_through_the_abi() {
    let original = frame();
    let bytes = encode_frame(&original);
    assert_eq!(bytes.len(), SVS_FRAME_LEN);
    let decoded = decode_frame(&bytes).expect("round-trips");
    assert_eq!(decoded, original);
}

#[test]
fn wrong_version_fails_closed() {
    let mut bytes = encode_frame(&frame());
    bytes[0] = bytes[0].wrapping_add(1);
    assert!(matches!(
        decode_frame(&bytes),
        Err(AbiError::BadVersion { .. })
    ));
    assert_eq!(u32::from_le_bytes([1, 0, 0, 0]), ABI_VERSION);
}

#[test]
fn truncated_buffer_fails_closed() {
    let bytes = encode_frame(&frame());
    assert!(matches!(
        decode_frame(&bytes[..SVS_FRAME_LEN - 1]),
        Err(AbiError::Truncated { .. })
    ));
    assert!(matches!(decode_frame(&[]), Err(AbiError::Truncated { .. })));
}

#[test]
fn unknown_enum_value_fails_closed() {
    // The horizontal-datum byte sits right after version(4) + lat(8) + lon(8).
    let mut bytes = encode_frame(&frame());
    bytes[20] = 200;
    match decode_frame(&bytes) {
        Err(AbiError::UnknownEnum { field, value }) => {
            assert_eq!(field, "horizontal_datum");
            assert_eq!(value, 200, "the actual unknown value is reported");
        }
        other => panic!("expected UnknownEnum, got {other:?}"),
    }
}

#[test]
fn non_finite_coordinate_fails_closed() {
    let mut bytes = encode_frame(&frame());
    // Overwrite the latitude (offset 4..12) with a NaN bit pattern.
    bytes[4..12].copy_from_slice(&f64::NAN.to_le_bytes());
    assert!(matches!(
        decode_frame(&bytes),
        Err(AbiError::NonFinite {
            field: "latitude_deg"
        })
    ));
}

#[test]
fn semantically_malformed_block_fails_closed() {
    // Change the vertical datum byte to MSL(2) while the geoid stays undeclared
    // for a frame whose vertical was built as Ellipsoid — decode must refuse.
    let vertical = VerticalPosition::new(
        250.0,
        VerticalDatum::Ellipsoid,
        GeoidModelId::UNDECLARED,
        LocalOriginId::UNDECLARED,
    )
    .expect("ellipsoid");
    let position = GeodeticPosition::new(1.0, 2.0, HorizontalDatum::Wgs84, vertical).expect("pos");
    let mut f = frame();
    f.position.position = position;
    let mut bytes = encode_frame(&f);
    // vertical-datum byte: version(4)+lat(8)+lon(8)+hdatum(1)+height(8) = 29.
    bytes[29] = VerticalDatum::Msl.to_u8();
    assert!(matches!(
        decode_frame(&bytes),
        Err(AbiError::Malformed { field: "vertical" })
    ));
}
