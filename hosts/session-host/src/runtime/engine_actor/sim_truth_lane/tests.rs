//! The truth lane: stamps, the geodetic fix the camera needs, resets, and
//! staleness.

#![allow(clippy::expect_used)]

use std::time::{Duration, Instant};

use pilotage_adapter_api::{MeasurementClock, SourceIncarnation, SourceRole};
use pilotage_geo::{HorizontalDatum, SIMULATOR_GEOID_MODEL_ID, VerticalDatum};

use super::{TruthMessage, TruthTracker};

fn message(time_usec: u64, lat_e7: i32, alt_mm: i32) -> TruthMessage {
    TruthMessage {
        time_usec,
        quat_wxyz: [0.6, 0.0, 0.0, 0.8],
        vel_ned_mps: [1.0, 0.0, -0.5],
        lat_lon_alt: [lat_e7, -747_381_918, alt_mm],
    }
}

#[test]
fn a_message_becomes_simulation_truth_with_an_msl_fix() {
    let mut tracker = TruthTracker::new(SourceIncarnation::new([7; 16]));
    let now = Instant::now();
    tracker.accept(message(4_000, 405_771_861, 56_419), now);
    let truth = tracker.latest(now).expect("a fresh truth");
    assert_eq!(truth.stamp.role, SourceRole::SimulationTruth);
    assert_eq!(truth.stamp.clock, MeasurementClock::Simulation);
    assert_eq!(truth.stamp.acquired_at_ns, 4_000_000);
    assert_eq!(truth.valid_flags & 0b101, 0b101, "attitude and position");
    assert_eq!(truth.quat_wxyz, [0.6, 0.0, 0.0, 0.8]);
    let fix = truth.geodetic.expect("a geodetic fix");
    assert_eq!(fix.position.horizontal_datum, HorizontalDatum::Wgs84);
    assert_eq!(fix.position.vertical.datum, VerticalDatum::Msl);
    assert_eq!(fix.position.vertical.geoid, SIMULATOR_GEOID_MODEL_ID);
    assert!((fix.position.latitude_deg - 40.577_186_1).abs() < 1e-9);
    assert!((fix.position.vertical.height_m - 56.419).abs() < 1e-9);
    assert_eq!(fix.stamp, truth.stamp);
}

#[test]
fn position_is_north_east_down_from_the_first_fix() {
    let mut tracker = TruthTracker::new(SourceIncarnation::new([7; 16]));
    let now = Instant::now();
    tracker.accept(message(4_000, 405_771_861, 56_000), now);
    // 0.0001 degree north (about 11.1 m) and 10 m up.
    tracker.accept(message(8_000, 405_772_861, 66_000), now);
    let truth = tracker.latest(now).expect("a fresh truth");
    assert!(
        (truth.pos_ned_m[0] - 11.1).abs() < 0.1,
        "{:?}",
        truth.pos_ned_m
    );
    assert!(truth.pos_ned_m[1].abs() < 1e-3);
    assert!((truth.pos_ned_m[2] + 10.0).abs() < 1e-3);
    assert_eq!(truth.stamp.sequence, 2);
}

#[test]
fn a_clock_that_steps_back_is_a_new_epoch_and_origin() {
    let mut tracker = TruthTracker::new(SourceIncarnation::new([7; 16]));
    let now = Instant::now();
    tracker.accept(message(900_000, 405_771_861, 56_000), now);
    tracker.accept(message(904_000, 405_772_861, 56_000), now);
    let before = tracker.latest(now).expect("a fresh truth");
    tracker.accept(message(4_000, 405_772_861, 56_000), now);
    let after = tracker.latest(now).expect("a fresh truth");
    assert_eq!(
        after.stamp.source_epoch,
        before.stamp.source_epoch.wrapping_add(1)
    );
    assert_eq!(after.pos_ned_m, [0.0, 0.0, 0.0]);
}

#[test]
fn truth_that_stopped_arriving_is_withheld() {
    let mut tracker = TruthTracker::new(SourceIncarnation::new([7; 16]));
    let then = Instant::now();
    assert!(tracker.latest(then).is_none());
    tracker.accept(message(4_000, 405_771_861, 56_000), then);
    assert!(tracker.latest(then + Duration::from_millis(400)).is_some());
    assert!(tracker.latest(then + Duration::from_secs(1)).is_none());
}
