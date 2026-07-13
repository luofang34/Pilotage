//! Round-trip, fail-closed, and hostile cross-field tests for the SVS frame
//! ABI. Availability is derived, never decoded: the central hostile case is
//! that an untrusted or incoherent input can never yield an available scene.
#![allow(clippy::expect_used, clippy::panic)]

use pilotage_frames::{ClockDomain, Epoch, Quat, TimeScale};

use super::{
    ABI_VERSION, RawSvsFrame, SVS_FRAME_LEN, ValidatedSvsFrame, decode_frame, encode_frame,
};
use crate::availability::{
    AvailabilityProfile, AvailabilityProfileId, AvailabilityReason, ExternalHealth, InputHealth,
    SvsAvailability,
};
use crate::datum::{
    BaroSettingId, DatumRealizationId, GeodeticPosition, GeoidModelId, HorizontalDatum,
    LocalOriginId, TerrainRefId, VerticalDatum, VerticalPosition,
};
use crate::error::AbiError;
use crate::identity::{
    AttitudeQuality, CoherentSnapshot, IntegrityLevel, PositionQuality, SourceIncarnation,
    SourceStamp, StatedAttitude, StatedPosition,
};
use crate::view::{
    CalibrationId, CalibrationRef, MinificationPolicy, NearFarPolicy, Projection, ProjectionView,
};

const ACQ_NS: u64 = 1_700_000_000_000_000_000;
const REF_NS: u64 = ACQ_NS + 10_000_000; // 10 ms after acquisition: fresh.

/// The evaluation profile every round-trip test judges against. Availability is
/// receiver-derived under this profile; the wire carries none.
const SIM: AvailabilityProfile = AvailabilityProfile::simulator();

fn epoch(nanos: u64) -> Epoch {
    Epoch {
        clock: ClockDomain::Gnss,
        scale: TimeScale::Gps,
        nanos,
    }
}

fn snapshot() -> CoherentSnapshot {
    CoherentSnapshot {
        producer: SourceIncarnation([9; 16]),
        generation: 4,
        id: 99,
    }
}

fn stamp(integrity: IntegrityLevel) -> SourceStamp {
    SourceStamp {
        source_id: 42,
        incarnation: SourceIncarnation([3; 16]),
        generation: 1,
        sequence: 7,
        acquired_at: epoch(ACQ_NS),
        integrity,
        snapshot: snapshot(),
    }
}

fn position(integrity: IntegrityLevel) -> StatedPosition {
    let vertical = VerticalPosition::new(
        250.0,
        VerticalDatum::Msl,
        GeoidModelId(1),
        TerrainRefId::UNDECLARED,
        BaroSettingId::UNDECLARED,
        LocalOriginId::UNDECLARED,
    )
    .expect("well-formed vertical");
    StatedPosition {
        position: GeodeticPosition::new(
            37.62,
            -122.38,
            HorizontalDatum::Wgs84,
            DatumRealizationId::UNDECLARED,
            vertical,
        )
        .expect("well-formed position"),
        stamp: stamp(integrity),
        quality: PositionQuality {
            horizontal_mm: 1500,
            vertical_mm: 3000,
        },
    }
}

fn attitude(integrity: IntegrityLevel) -> StatedAttitude {
    StatedAttitude {
        attitude: Quat::IDENTITY,
        stamp: stamp(integrity),
        quality: AttitudeQuality { angular_mrad: 5 },
    }
}

fn view() -> ProjectionView {
    ProjectionView {
        calibration: CalibrationRef {
            calibration_id: CalibrationId(0x0FED_CBA9),
            content_hash: [7u8; 32],
        },
        projection: Projection::Perspective,
        near_far: NearFarPolicy {
            near_m: 0.1,
            far_m: 20_000.0,
        },
        minification: MinificationPolicy::Trilinear,
    }
}

fn external_ok() -> ExternalHealth {
    let ok = InputHealth::Ok;
    ExternalHealth {
        integrity: ok,
        calibration: ok,
        database: ok,
        coverage: ok,
        renderer: ok,
    }
}

/// A fully trusted, fresh, coherent raw frame — the only shape that derives an
/// available scene.
fn raw() -> RawSvsFrame {
    RawSvsFrame {
        position: position(IntegrityLevel::Trusted),
        attitude: attitude(IntegrityLevel::Trusted),
        view: view(),
        external: external_ok(),
        reference_time: epoch(REF_NS),
    }
}

fn validated() -> ValidatedSvsFrame {
    raw().validate(&SIM).expect("the nominal frame validates")
}

#[test]
fn frame_round_trips_through_the_abi_and_derives_availability() {
    let original = validated();
    assert_eq!(original.availability(), SvsAvailability::Available);
    let bytes = encode_frame(&original);
    assert_eq!(bytes.len(), SVS_FRAME_LEN);
    let decoded = decode_frame(&bytes, &SIM).expect("round-trips");
    assert_eq!(decoded, original);
    assert_eq!(
        decoded.availability(),
        SvsAvailability::Available,
        "availability is recomputed identically on decode"
    );
}

#[test]
fn availability_is_never_read_from_the_wire() {
    // Untrusted position integrity: the derived verdict is Unavailable no matter
    // what a producer might wish. There is no wire byte a producer could set to
    // claim Available over this input.
    let mut r = raw();
    r.position = position(IntegrityLevel::Untrusted);
    let f = r
        .validate(&SIM)
        .expect("structurally valid, just untrusted");
    assert_eq!(
        f.availability(),
        SvsAvailability::Unavailable(AvailabilityReason::Position),
    );
    // Round-trips as still Unavailable — the wire carried no availability.
    let decoded = decode_frame(&encode_frame(&f), &SIM).expect("round-trips");
    assert_eq!(
        decoded.availability(),
        SvsAvailability::Unavailable(AvailabilityReason::Position),
    );
}

#[test]
fn unknown_integrity_input_is_never_available() {
    let mut r = raw();
    r.attitude = attitude(IntegrityLevel::Unknown);
    let f = r.validate(&SIM).expect("structurally valid");
    assert_eq!(
        f.availability(),
        SvsAvailability::Unavailable(AvailabilityReason::Attitude),
    );
}

#[test]
fn a_non_unit_aircraft_attitude_is_refused() {
    // validate() refuses it structurally.
    let mut r = raw();
    r.attitude.attitude = Quat {
        w: 2.0,
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    assert!(matches!(
        r.validate(&SIM),
        Err(crate::error::GeoError::AttitudeNotARotation)
    ));
    // And decode refuses a wire frame whose attitude quaternion is not unit.
    let mut bytes = encode_frame(&validated());
    // Aircraft quaternion w is the first f32 of the attitude block at offset
    // version(4) + position(125) = 129.
    bytes[129..133].copy_from_slice(&2.0f32.to_le_bytes());
    assert!(matches!(
        decode_frame(&bytes, &SIM),
        Err(AbiError::Malformed {
            field: "attitude",
            ..
        })
    ));
}

#[test]
fn a_future_acquisition_fails_time_coherence() {
    let mut r = raw();
    r.reference_time = epoch(ACQ_NS - 1); // before acquisition: a future sample.
    let f = r.validate(&SIM).expect("structurally valid");
    assert_eq!(
        f.availability(),
        SvsAvailability::Unavailable(AvailabilityReason::TimeCoherence),
    );
}

#[test]
fn a_snapshot_id_collision_across_streams_is_not_coherent() {
    // Position and attitude share the numeric snapshot id but from different
    // producers: not one coherent snapshot, so the scene is unavailable.
    let mut r = raw();
    r.attitude.stamp.snapshot.producer = SourceIncarnation([1; 16]);
    let f = r.validate(&SIM).expect("structurally valid");
    assert_eq!(
        f.availability(),
        SvsAvailability::Unavailable(AvailabilityReason::TimeCoherence),
    );
}

#[test]
fn a_clock_or_scale_mismatch_fails_time_coherence() {
    let mut r = raw();
    r.attitude.stamp.acquired_at = Epoch {
        clock: ClockDomain::Simulation,
        scale: TimeScale::Monotonic,
        nanos: ACQ_NS,
    };
    let f = r.validate(&SIM).expect("structurally valid");
    assert_eq!(
        f.availability(),
        SvsAvailability::Unavailable(AvailabilityReason::TimeCoherence),
    );
}

#[test]
fn wrong_length_fails_closed_including_trailing_bytes() {
    let bytes = encode_frame(&validated());
    assert!(matches!(
        decode_frame(&bytes[..SVS_FRAME_LEN - 1], &SIM),
        Err(AbiError::WrongLength { .. })
    ));
    assert!(matches!(
        decode_frame(&[], &SIM),
        Err(AbiError::WrongLength { .. })
    ));
    // Trailing bytes are as suspect as truncation for a fixed-size block.
    let mut too_long = bytes.to_vec();
    too_long.push(0);
    assert!(
        matches!(
            decode_frame(&too_long, &SIM),
            Err(AbiError::WrongLength { .. })
        ),
        "a fixed block must match its length exactly"
    );
}

#[test]
fn wrong_version_fails_closed() {
    let mut bytes = encode_frame(&validated());
    bytes[0] = bytes[0].wrapping_add(1);
    assert!(matches!(
        decode_frame(&bytes, &SIM),
        Err(AbiError::BadVersion { .. })
    ));
    assert_eq!(u32::from_le_bytes([2, 0, 0, 0]), ABI_VERSION);
}

#[test]
fn unknown_enum_value_fails_closed() {
    // The horizontal-datum byte sits right after version(4) + lat(8) + lon(8).
    let mut bytes = encode_frame(&validated());
    bytes[20] = 200;
    match decode_frame(&bytes, &SIM) {
        Err(AbiError::UnknownEnum { field, value }) => {
            assert_eq!(field, "horizontal_datum");
            assert_eq!(value, 200, "the actual unknown value is reported");
        }
        other => panic!("expected UnknownEnum, got {other:?}"),
    }
}

#[test]
fn non_finite_coordinate_fails_closed() {
    let mut bytes = encode_frame(&validated());
    // Overwrite the latitude (offset 4..12) with a NaN bit pattern.
    bytes[4..12].copy_from_slice(&f64::NAN.to_le_bytes());
    assert!(matches!(
        decode_frame(&bytes, &SIM),
        Err(AbiError::NonFinite {
            field: "latitude_deg"
        })
    ));
}

#[test]
fn an_incomplete_datum_identity_fails_closed() {
    // Flip the vertical datum to AGL while the terrain reference stays
    // undeclared (the nominal frame is MSL with a geoid); decode must refuse.
    let mut bytes = encode_frame(&validated());
    // vdatum byte: version(4)+lat(8)+lon(8)+hdatum(1)+realization(2)+height(8) = 31.
    bytes[31] = VerticalDatum::Agl.to_u8();
    assert!(matches!(
        decode_frame(&bytes, &SIM),
        Err(AbiError::Malformed {
            field: "vertical",
            ..
        })
    ));
}

#[test]
fn an_incomplete_calibration_reference_fails_closed() {
    let mut r = raw();
    r.view.calibration.calibration_id = CalibrationId::NONE;
    assert!(matches!(
        r.validate(&SIM),
        Err(crate::error::GeoError::IncompleteCalibrationReference)
    ));
    // Decode refuses a zero calibration id on the wire (u32 id at view offset:
    // version(4)+position(125)+attitude(91) = 220).
    let mut bytes = encode_frame(&validated());
    bytes[220..224].copy_from_slice(&0u32.to_le_bytes());
    assert!(matches!(
        decode_frame(&bytes, &SIM),
        Err(AbiError::Malformed { field: "view", .. })
    ));
}

#[test]
fn an_orthographic_view_without_extents_fails_closed() {
    let mut r = raw();
    r.view.projection = Projection::Orthographic {
        extent_x_m: 0.0,
        extent_y_m: 375.0,
    };
    assert!(matches!(
        r.validate(&SIM),
        Err(crate::error::GeoError::InvalidOrthographicExtent { .. })
    ));
}

#[test]
fn an_orthographic_frame_is_not_read_as_perspective() {
    let mut r = raw();
    r.view.projection = Projection::Orthographic {
        extent_x_m: 500.0,
        extent_y_m: 375.0,
    };
    let f = r.validate(&SIM).expect("valid orthographic frame");
    let decoded = decode_frame(&encode_frame(&f), &SIM).expect("round-trips");
    assert_eq!(
        decoded.view().projection,
        Projection::Orthographic {
            extent_x_m: 500.0,
            extent_y_m: 375.0,
        },
        "the projection kind byte selects the payload; it is not silently perspective",
    );
}

#[test]
fn an_unknown_external_health_byte_is_refused_not_coerced() {
    // External health sits just before the 10-byte reference time; the first of
    // its five bytes is the navigation-integrity monitor. An unknown value must
    // be refused with UnknownEnum, not silently coerced to Failed — otherwise
    // decode-then-encode would change the bytes.
    let mut bytes = encode_frame(&validated());
    let external_off = SVS_FRAME_LEN - 5 - 10;
    bytes[external_off] = 200;
    match decode_frame(&bytes, &SIM) {
        Err(AbiError::UnknownEnum { field, value }) => {
            assert_eq!(field, "external_integrity");
            assert_eq!(value, 200);
        }
        other => panic!("expected UnknownEnum, got {other:?}"),
    }
}

#[test]
fn an_encoded_frame_can_only_come_from_a_validated_one() {
    // encode_frame's signature takes &ValidatedSvsFrame, and the only ways to
    // obtain one are validate()/decode() — both of which enforce every
    // structural invariant. This test documents the guarantee and pins that a
    // decoded frame re-encodes byte-identically.
    let f = validated();
    let once = encode_frame(&f);
    let twice = encode_frame(&decode_frame(&once, &SIM).expect("round-trips"));
    assert_eq!(once, twice);
}

#[test]
fn the_evaluation_profile_is_bound_in_and_survives_round_trip() {
    let f = validated();
    assert_eq!(f.profile(), SIM);
    let decoded = decode_frame(&encode_frame(&f), &SIM).expect("round-trips");
    assert_eq!(decoded.profile(), SIM);
    assert_eq!(decoded, f);
}

#[test]
fn the_same_wire_bytes_are_judged_by_the_receivers_profile() {
    // One producer frame, fresh and trusted, encodes once. A receiver that
    // evaluates the identical bytes under a stricter position-accuracy profile
    // gets a different, traceable verdict: the profile is receiver-controlled
    // evaluation context, not wire data, so ABI v2 is unchanged.
    let bytes = encode_frame(&validated());

    let strict = AvailabilityProfile::new(
        AvailabilityProfileId(2),
        1,
        SIM.fresh_age_ns(),
        SIM.usable_age_ns(),
        1_000,
        2_000,
        SIM.fresh_att_mrad(),
        SIM.usable_att_mrad(),
    )
    .expect("a monotonic profile");

    let under_sim = decode_frame(&bytes, &SIM).expect("decodes under sim");
    let under_strict = decode_frame(&bytes, &strict).expect("decodes under strict");

    // Identical bytes decode to identical position, attitude, and view...
    assert_eq!(under_sim.position(), under_strict.position());
    assert_eq!(under_sim.attitude(), under_strict.attitude());
    assert_eq!(under_sim.view(), under_strict.view());
    // ...but a different, profile-traceable availability verdict.
    assert_eq!(under_sim.availability(), SvsAvailability::Available);
    assert_eq!(under_sim.profile(), SIM);
    assert_eq!(
        under_strict.availability(),
        SvsAvailability::Unavailable(AvailabilityReason::Position),
    );
    assert_eq!(under_strict.profile(), strict);

    // The wire bytes are profile-independent: re-encoding either verdict yields
    // exactly the original block, so no profile or availability ever reaches the
    // wire and ABI v2 bytes are untouched by the profile.
    assert_eq!(encode_frame(&under_sim), bytes);
    assert_eq!(encode_frame(&under_strict), bytes);
}
