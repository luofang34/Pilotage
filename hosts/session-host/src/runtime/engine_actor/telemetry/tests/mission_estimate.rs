//! A sample shaped as the DJI adapter publishes under a declared heading
//! reference reaches the mission's estimate path through the wire.

use navigate_contract::MonotonicNanos;
use pilotage_adapter_api::{
    AvionicsAttitudeSample, AvionicsKinematicsSample, AvionicsSample, GeodeticFixSample,
    MeasurementClock, MeasurementStamp, SourceIncarnation, SourceIntegrity, SourceRole,
};
use pilotage_geo::{
    BaroSettingId, DatumRealizationId, GeodeticPosition, GeoidModelId, HorizontalDatum,
    LocalOriginId, PositionQuality, TerrainRefId, VerticalDatum, VerticalPosition,
};
use pilotage_mission::OwnshipAltitude;
use pilotage_timing::MonoTimestamp;

use super::super::sample_to_wire;
use super::one_vehicle_sample;
use crate::runtime::automation::ownship::estimate_from_wire;

const HEIGHT_M: f64 = 12.0;

/// A canonical attitude at `yaw` and a local-relative fix under one
/// estimate stamp, as the DJI airframe publishes them.
fn dji_shaped(yaw: f32) -> AvionicsSample {
    let estimate = MeasurementStamp {
        role: SourceRole::OperationalEstimate,
        integrity: SourceIntegrity::ChecksummedOnly,
        source_id: 0x0e_03_43,
        source_incarnation: SourceIncarnation::new([7; 16]),
        source_epoch: 0,
        sequence: 3,
        acquired_at_ns: 1_000,
        clock: MeasurementClock::HostMonotonic,
    };
    let vertical = VerticalPosition::new(
        HEIGHT_M,
        VerticalDatum::LocalRelative,
        GeoidModelId::UNDECLARED,
        TerrainRefId::UNDECLARED,
        BaroSettingId::UNDECLARED,
        LocalOriginId(77),
    )
    .expect("a declared local height");
    let position = GeodeticPosition::new(
        40.5442,
        -74.4565,
        HorizontalDatum::Wgs84,
        DatumRealizationId::UNDECLARED,
        vertical,
    )
    .expect("a plausible fix");
    AvionicsSample {
        attitude: Some(AvionicsAttitudeSample::Canonical {
            quat_wxyz: [(yaw / 2.0).cos(), 0.0, 0.0, (yaw / 2.0).sin()],
            rates_rps: None,
            stamp: estimate,
        }),
        kinematics: Some(AvionicsKinematicsSample {
            pos_ned_m: [0.0, 0.0, -HEIGHT_M as f32],
            vel_ned_mps: [1.0, 0.0, 0.0],
            stamp: estimate,
        }),
        baro: None,
        geodetic: Some(GeodeticFixSample {
            position,
            quality: PositionQuality {
                horizontal_mm: 0,
                vertical_mm: 0,
            },
            stamp: estimate,
        }),
        magnetic_variation: None,
        estimator_status_stamp: Some(estimate),
        valid_flags: 0b1101,
        quality: 0,
    }
}

#[test]
fn a_declared_heading_reaches_the_mission_with_its_fix() {
    let yaw = 0.4_f32;
    let wire_sample = sample_to_wire(
        one_vehicle_sample(dji_shaped(yaw)),
        MonoTimestamp::from_nanos(2_000),
    );
    let ownship = estimate_from_wire(
        &wire_sample,
        MonotonicNanos::from_nanos(2_000),
        |_, _, altitude| match altitude {
            OwnshipAltitude::AboveAnchor(height_m) => [0.0, 0.0, -height_m],
            OwnshipAltitude::Ellipsoid(altitude_m) => [0.0, 0.0, -altitude_m - 1_000.0],
        },
    )
    .expect("the authorized attitude and fix are admitted together");
    assert!((ownship.yaw_rad.expect("heading") - f64::from(yaw)).abs() < 1e-5);
    assert_eq!(
        ownship.ned[2], -HEIGHT_M,
        "a local-relative height is height above the anchor"
    );
    assert_eq!(ownship.sequence, 3);
}
