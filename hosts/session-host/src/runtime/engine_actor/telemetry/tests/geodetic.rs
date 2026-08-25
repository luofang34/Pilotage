//! The geodetic fix on the wire (ADR-0022): what an absent fix does, what
//! a fix carries, and what the mapping refuses to send.

use pilotage_adapter_api::{
    AvionicsSample, MeasurementClock, MeasurementStamp, SimTruthSample, SourceIncarnation,
    SourceIntegrity, SourceRole,
};

/// One acquisition stamp, for the tests that only need a valid one.
fn truth_stamp() -> MeasurementStamp {
    MeasurementStamp {
        role: SourceRole::SimulationTruth,
        integrity: SourceIntegrity::ChecksummedOnly,
        source_id: 11,
        source_incarnation: SourceIncarnation::new([0x5A; 16]),
        source_epoch: 1,
        sequence: 4,
        acquired_at_ns: 9_000_000,
        clock: MeasurementClock::VehicleBoot,
    }
}

/// An absent fix must stay absent on the wire. 0,0 is a real place in the
/// Gulf of Guinea: a substituted zero would draw a plausible vehicle
/// there, which is the failure ADR-0022 exists to prevent.
#[test]
fn an_absent_geodetic_fix_is_absent_and_never_null_island() {
    let stamp = truth_stamp();
    let avionics = super::super::avionics_to_wire(AvionicsSample {
        geodetic: None,
        attitude: None,
        kinematics: None,
        baro: None,
        estimator_status_stamp: None,
        valid_flags: 0,
        quality: 0,
    });
    assert!(
        avionics.geodetic.is_none(),
        "no fix supplied means no fix on the wire"
    );
    assert!(avionics.geodetic_stamp.is_none());

    let truth = super::super::sim_truth_to_wire(SimTruthSample {
        geodetic: None,
        quat_wxyz: [1.0, 0.0, 0.0, 0.0],
        pos_ned_m: [0.0; 3],
        vel_ned_mps: [0.0; 3],
        valid_flags: 0,
        stamp,
    });
    assert!(
        truth.geodetic.is_none(),
        "an oracle with no declared position declares none"
    );
}

/// A fix crosses with every datum identity it needs, and the reader can
/// rebuild the typed value from the wire without inferring anything.
#[test]
fn a_geodetic_fix_carries_its_whole_datum() {
    let stamp = truth_stamp();
    let vertical = pilotage_geo::VerticalPosition::new(
        488.227,
        pilotage_geo::VerticalDatum::Msl,
        pilotage_geo::SIMULATOR_GEOID_MODEL_ID,
        pilotage_geo::TerrainRefId::UNDECLARED,
        pilotage_geo::BaroSettingId::UNDECLARED,
        pilotage_geo::LocalOriginId::UNDECLARED,
    )
    .expect("a simulator height declares its separation");
    let position = pilotage_geo::GeodeticPosition::new(
        47.397_741_9,
        8.545_593_8,
        pilotage_geo::HorizontalDatum::Wgs84,
        pilotage_geo::DatumRealizationId::UNDECLARED,
        vertical,
    )
    .expect("WGS-84 needs no realization");
    let truth = super::super::sim_truth_to_wire(SimTruthSample {
        geodetic: Some(pilotage_adapter_api::GeodeticFixSample {
            position,
            quality: pilotage_geo::PositionQuality {
                horizontal_mm: 1_500,
                vertical_mm: 3_000,
            },
            stamp,
        }),
        quat_wxyz: [1.0, 0.0, 0.0, 0.0],
        pos_ned_m: [0.0; 3],
        vel_ned_mps: [0.0; 3],
        valid_flags: 0,
        stamp,
    });
    let fix = truth.geodetic.expect("the fix crosses");
    assert!((fix.latitude_deg - 47.397_741_9).abs() < 1e-9);
    assert!((fix.longitude_deg - 8.545_593_8).abs() < 1e-9);
    assert_eq!(
        fix.horizontal_datum,
        u32::from(pilotage_geo::HorizontalDatum::Wgs84.to_u8())
    );
    assert_eq!(
        fix.vertical_datum,
        u32::from(pilotage_geo::VerticalDatum::Msl.to_u8())
    );
    assert_eq!(
        fix.geoid_model,
        u32::from(pilotage_geo::SIMULATOR_GEOID_MODEL_ID.0),
        "an MSL height names the separation it was measured from"
    );
    assert_eq!(fix.horizontal_accuracy_mm, 1_500);
    assert_eq!(fix.vertical_accuracy_mm, 3_000);

    // The value the receiver rebuilds is the value that was sent.
    let rebuilt = pilotage_geo::GeodeticPosition::new(
        fix.latitude_deg,
        fix.longitude_deg,
        pilotage_geo::HorizontalDatum::from_u8(fix.horizontal_datum as u8)
            .expect("a declared datum"),
        pilotage_geo::DatumRealizationId(fix.realization as u16),
        pilotage_geo::VerticalPosition::new(
            fix.height_m,
            pilotage_geo::VerticalDatum::from_u8(fix.vertical_datum as u8)
                .expect("a declared vertical datum"),
            pilotage_geo::GeoidModelId(fix.geoid_model as u16),
            pilotage_geo::TerrainRefId(fix.terrain_ref),
            pilotage_geo::BaroSettingId(fix.baro_setting),
            pilotage_geo::LocalOriginId(fix.local_origin),
        )
        .expect("the vertical datum survives the wire"),
    )
    .expect("the horizontal datum survives the wire");
    assert_eq!(rebuilt, position);
}

/// The mapping is the last place before the wire, and the typed value has
/// public fields, so a producer can assemble one the constructor would
/// have refused. A position that cannot be interpreted must not be sent.
#[test]
fn an_assembled_position_the_contract_refuses_never_reaches_the_wire() {
    let stamp = truth_stamp();
    let vertical = pilotage_geo::VerticalPosition {
        height_m: 100.0,
        datum: pilotage_geo::VerticalDatum::Msl,
        // An MSL height that names no separation is uninterpretable.
        geoid: pilotage_geo::GeoidModelId::UNDECLARED,
        terrain_ref: pilotage_geo::TerrainRefId::UNDECLARED,
        baro_setting: pilotage_geo::BaroSettingId::UNDECLARED,
        origin: pilotage_geo::LocalOriginId::UNDECLARED,
    };
    let position = pilotage_geo::GeodeticPosition {
        latitude_deg: 47.0,
        longitude_deg: 8.0,
        horizontal_datum: pilotage_geo::HorizontalDatum::Wgs84,
        realization: pilotage_geo::DatumRealizationId::UNDECLARED,
        vertical,
    };
    let truth = super::super::sim_truth_to_wire(SimTruthSample {
        geodetic: Some(pilotage_adapter_api::GeodeticFixSample {
            position,
            quality: pilotage_geo::PositionQuality {
                horizontal_mm: 0,
                vertical_mm: 0,
            },
            stamp,
        }),
        quat_wxyz: [1.0, 0.0, 0.0, 0.0],
        pos_ned_m: [0.0; 3],
        vel_ned_mps: [0.0; 3],
        valid_flags: 0,
        stamp,
    });
    assert!(
        truth.geodetic.is_none(),
        "a position the contract refuses is not sent"
    );
}

/// The simulator's declared separation names no geoid. Under an
/// operational role it would read as a surveyed height measured from a
/// model nothing can name.
#[test]
fn the_simulator_separation_cannot_ride_an_operational_role() {
    let vertical = pilotage_geo::VerticalPosition::new(
        488.0,
        pilotage_geo::VerticalDatum::Msl,
        pilotage_geo::SIMULATOR_GEOID_MODEL_ID,
        pilotage_geo::TerrainRefId::UNDECLARED,
        pilotage_geo::BaroSettingId::UNDECLARED,
        pilotage_geo::LocalOriginId::UNDECLARED,
    )
    .expect("a simulator height declares its separation");
    let position = pilotage_geo::GeodeticPosition::new(
        47.0,
        8.0,
        pilotage_geo::HorizontalDatum::Wgs84,
        pilotage_geo::DatumRealizationId::UNDECLARED,
        vertical,
    )
    .expect("WGS-84 needs no realization");
    let fix = |role| pilotage_adapter_api::GeodeticFixSample {
        position,
        quality: pilotage_geo::PositionQuality {
            horizontal_mm: 0,
            vertical_mm: 0,
        },
        stamp: MeasurementStamp {
            role,
            ..truth_stamp()
        },
    };
    let estimate = super::super::avionics_to_wire(AvionicsSample {
        geodetic: Some(fix(SourceRole::OperationalEstimate)),
        attitude: None,
        kinematics: None,
        baro: None,
        estimator_status_stamp: None,
        valid_flags: 0,
        quality: 0,
    });
    assert!(
        estimate.geodetic.is_none(),
        "an operational estimate cannot carry the simulator separation"
    );
    assert!(
        estimate.geodetic_stamp.is_none(),
        "a stamp beside no position would claim an observation the wire does not carry"
    );

    let truth = super::super::sim_truth_to_wire(SimTruthSample {
        geodetic: Some(fix(SourceRole::SimulationTruth)),
        quat_wxyz: [1.0, 0.0, 0.0, 0.0],
        pos_ned_m: [0.0; 3],
        vel_ned_mps: [0.0; 3],
        valid_flags: 0,
        stamp: truth_stamp(),
    });
    assert!(
        truth.geodetic.is_some(),
        "the oracle's own separation rides the oracle's own role"
    );
}
