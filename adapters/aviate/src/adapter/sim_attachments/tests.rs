//! Joining a satellite-navigation fix to the truth observation it belongs
//! with. The two arrive over different transports, so the simulation clock
//! is what says whether they describe the same moment.

#![allow(clippy::expect_used, clippy::panic)]

use pilotage_adapter_api::{
    MeasurementClock, MeasurementStamp, SourceIncarnation, SourceIntegrity, SourceRole,
};
use pilotage_geo::{HorizontalDatum, SIMULATOR_GEOID_MODEL_ID, VerticalDatum};
use pilotage_sim_video::BridgeNavSat;

use super::fix_for_moment;

/// The datum both simulator worlds declare.
const REPORTED: (f64, f64, f64) = (47.397_741_9, 8.545_593_8, 488.227);

fn stamp_at(acquired_at_ns: u64) -> MeasurementStamp {
    MeasurementStamp {
        role: SourceRole::SimulationTruth,
        integrity: SourceIntegrity::Unprotected,
        source_id: 1,
        source_incarnation: SourceIncarnation::new([7; 16]),
        source_epoch: 0,
        sequence: 5,
        acquired_at_ns,
        clock: MeasurementClock::Simulation,
    }
}

fn fix_at(sim_time_ns: u64) -> BridgeNavSat {
    BridgeNavSat {
        latitude_deg: REPORTED.0,
        longitude_deg: REPORTED.1,
        altitude_m: REPORTED.2,
        sim_time_ns,
    }
}

#[test]
fn a_fix_from_the_same_moment_joins_the_observation() {
    let stamp = stamp_at(5_000_000_000);
    let joined = fix_for_moment(fix_at(5_000_000_000), stamp).expect("the same moment joins");
    assert!((joined.position.latitude_deg - REPORTED.0).abs() < 1e-9);
    assert!((joined.position.longitude_deg - REPORTED.1).abs() < 1e-9);
    assert!((joined.position.vertical.height_m - REPORTED.2).abs() < 1e-6);
    assert_eq!(joined.position.horizontal_datum, HorizontalDatum::Wgs84);
    assert_eq!(joined.position.vertical.datum, VerticalDatum::Msl);
    assert_eq!(
        joined.position.vertical.geoid, SIMULATOR_GEOID_MODEL_ID,
        "a simulated height names the separation it came from"
    );
    assert_eq!(
        joined.stamp, stamp,
        "the fix rides the observation it was joined to"
    );
    assert_eq!(
        joined.quality.horizontal_mm, 0,
        "the simulator states no accuracy, and unstated is what the wire reads"
    );
}

#[test]
fn a_fix_from_another_moment_is_refused() {
    let stamp = stamp_at(5_000_000_000);
    // A sensor period either side still describes the same moment.
    assert!(fix_for_moment(fix_at(5_033_000_000), stamp).is_some());
    assert!(fix_for_moment(fix_at(4_967_000_000), stamp).is_some());
    // Half a second later is the vehicle somewhere else.
    assert!(
        fix_for_moment(fix_at(5_500_000_000), stamp).is_none(),
        "a stale fix would place the vehicle where it used to be"
    );
    assert!(fix_for_moment(fix_at(4_500_000_000), stamp).is_none());
}

#[test]
fn a_sensor_that_has_not_spoken_states_no_position() {
    let stamp = stamp_at(0);
    let silent = BridgeNavSat {
        latitude_deg: 0.0,
        longitude_deg: 0.0,
        altitude_m: 0.0,
        sim_time_ns: 0,
    };
    assert!(
        fix_for_moment(silent, stamp).is_none(),
        "0,0 is a real place, so only this side can tell the two apart"
    );
}

#[test]
fn a_position_the_contract_refuses_is_no_position() {
    let stamp = stamp_at(1_000);
    let past_the_pole = BridgeNavSat {
        latitude_deg: 91.0,
        longitude_deg: 8.5,
        altitude_m: 400.0,
        sim_time_ns: 1_000,
    };
    assert!(fix_for_moment(past_the_pole, stamp).is_none());
}

/// A world that declares no datum leaves the sensor's origin at zero, and a
/// vehicle standing on the ground there reports a small non-zero altitude.
/// Requiring the altitude to be zero as well let that whole case through,
/// and the map drew a vehicle off the coast of Africa.
#[test]
fn a_world_with_no_datum_states_no_position() {
    let fix = pilotage_sim_video::BridgeNavSat {
        latitude_deg: 0.0,
        longitude_deg: 0.0,
        // Standing on the ground in a world whose origin nobody set.
        altitude_m: 0.193,
        sim_time_ns: 1_000_000,
    };
    assert!(
        super::fix_for_moment(fix, stamp_at(1_000_000)).is_none(),
        "zero on both angles is the default nobody set, not a place"
    );
}
