//! The simulator oracle's position on the Earth, as it reaches the sample.
//!
//! The link projects each truth report into the local frame against an
//! origin it latches from the first report. The projection is one-way
//! without that origin, and a map needs the position itself, so the report
//! travels beside the projection rather than being replaced by it. These
//! checks pin that the position that arrives is the one the simulator sent,
//! under the truth role, with a datum a reader can interpret.

#![allow(clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pilotage_adapter_api::SourceRole;
use pilotage_geo::{HorizontalDatum, SIMULATOR_GEOID_MODEL_ID, VerticalDatum};
use pilotage_mavlink::link::{LinkState, SimTruthUpdate};

use super::sim_truth_sample;

/// Zurich, the datum the flight-deck world declares, at 488.227 m.
const REPORTED: [i32; 3] = [473_977_419, 85_455_938, 488_227];

fn state_with_truth(lat_lon_alt: [i32; 3], age: Duration) -> Arc<Mutex<LinkState>> {
    let state = LinkState {
        source_id: 4,
        sim_truth: Some(SimTruthUpdate {
            quat_wxyz: [1.0, 0.0, 0.0, 0.0],
            pos_ned_m: [0.0; 3],
            vel_ned_mps: [0.0; 3],
            lat_lon_alt,
            time_usec: 1_000,
            sequence: 3,
            received_at: Instant::now() - age,
        }),
        ..LinkState::default()
    };
    Arc::new(Mutex::new(state))
}

#[test]
fn the_oracle_reports_the_position_the_simulator_sent() {
    let state = state_with_truth(REPORTED, Duration::ZERO);
    let latest = state.lock().expect("link state");
    let sample = sim_truth_sample(&latest).expect("a fresh truth sample");
    let fix = sample.geodetic.expect("the oracle declares a position");

    assert!((fix.position.latitude_deg - 47.397_741_9).abs() < 1e-9);
    assert!((fix.position.longitude_deg - 8.545_593_8).abs() < 1e-9);
    assert!((fix.position.vertical.height_m - 488.227).abs() < 1e-6);
    assert_eq!(fix.position.horizontal_datum, HorizontalDatum::Wgs84);
    assert_eq!(fix.position.vertical.datum, VerticalDatum::Msl);
    assert_eq!(
        fix.position.vertical.geoid, SIMULATOR_GEOID_MODEL_ID,
        "a simulated MSL height names the separation it came from"
    );
}

#[test]
fn the_fix_rides_the_truth_role_and_the_samples_own_stamp() {
    let state = state_with_truth(REPORTED, Duration::ZERO);
    let latest = state.lock().expect("link state");
    let sample = sim_truth_sample(&latest).expect("a fresh truth sample");
    let fix = sample.geodetic.expect("the oracle declares a position");

    assert_eq!(
        fix.stamp.role,
        SourceRole::SimulationTruth,
        "an oracle position is never an operational estimate"
    );
    assert_eq!(
        fix.stamp, sample.stamp,
        "the fix and the projection are one observation, so they share a stamp"
    );
}

#[test]
fn a_position_the_contract_refuses_is_no_position() {
    // A latitude past the pole cannot be a place. The sample keeps its
    // local frame; it does not carry a position the reader would draw.
    let state = state_with_truth([910_000_000, 0, 0], Duration::ZERO);
    let latest = state.lock().expect("link state");
    let sample = sim_truth_sample(&latest).expect("a fresh truth sample");
    assert!(
        sample.geodetic.is_none(),
        "a refused report leaves no position behind"
    );
    assert_eq!(sample.valid_flags, 0b1101, "the local frame still arrives");
}

#[test]
fn a_stale_report_carries_no_position_because_it_carries_no_sample() {
    let state = state_with_truth(REPORTED, Duration::from_secs(5));
    let latest = state.lock().expect("link state");
    assert!(
        sim_truth_sample(&latest).is_none(),
        "a withheld truth report publishes nothing at all"
    );
}

#[test]
fn a_report_that_states_no_position_is_not_null_island() {
    // A truth frame whose whole geodetic triple is zero is a simulator
    // that has not stated a position. 0,0 is a real place off the coast of
    // Africa, and the typed contract accepts it, so nothing below this
    // point could tell the two apart.
    let state = state_with_truth([0, 0, 0], Duration::ZERO);
    let latest = state.lock().expect("link state");
    let sample = sim_truth_sample(&latest).expect("a fresh truth sample");
    assert!(
        sample.geodetic.is_none(),
        "a simulator that stated no position declares none"
    );
    assert_eq!(sample.valid_flags, 0b1101, "the local frame still arrives");
}
