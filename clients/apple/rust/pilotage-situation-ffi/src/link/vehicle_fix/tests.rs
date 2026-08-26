//! What the map is allowed to draw from a telemetry sample.
//!
//! These are the browser's rules restated against the same corpus values, so
//! a change that moves one client and not the other fails here.

#![allow(clippy::expect_used, clippy::panic)]

use pilotage_protocol::wire;

use super::from_sample;

/// Bits 0 and 3: attitude and velocity stated.
const ATTITUDE_AND_VELOCITY: u32 = 0b1001;

fn stamp() -> wire::MeasurementStamp {
    wire::MeasurementStamp {
        sequence: 1,
        ..Default::default()
    }
}

fn fix() -> wire::GeodeticFix {
    wire::GeodeticFix {
        latitude_deg: 47.397_742,
        longitude_deg: 8.545_594,
        horizontal_datum: 1,
        ..Default::default()
    }
}

/// A truth lane facing north at `speed`, level.
fn truth(speed_north: f32, speed_east: f32) -> wire::TelemetrySample {
    wire::TelemetrySample {
        sim_truth: Some(Box::new(wire::SimTruthState {
            quat_w: 1.0,
            quat_x: 0.0,
            quat_y: 0.0,
            quat_z: 0.0,
            vel_n_mps: speed_north,
            vel_e_mps: speed_east,
            valid_flags: ATTITUDE_AND_VELOCITY,
            stamp: Some(stamp()),
            geodetic: Some(fix()),
            ..Default::default()
        })),
        ..Default::default()
    }
}

#[test]
fn a_sample_with_no_lane_states_no_position() {
    // Absence is not the origin. A map that draws 0,0 for "nothing known"
    // puts the vehicle in the Gulf of Guinea.
    assert!(from_sample(&wire::TelemetrySample::default()).is_none());
}

#[test]
fn truth_without_its_stamp_is_unconsumable() {
    // A position that cannot be shown to be this vehicle's, this boot's and
    // this moment's is not drawn, however plausible its numbers.
    let mut sample = truth(3.0, 0.0);
    sample.sim_truth.as_mut().expect("truth lane").stamp = None;
    assert!(from_sample(&sample).is_none());
}

#[test]
fn the_oracle_is_preferred_and_says_so() {
    let read = from_sample(&truth(3.0, 0.0)).expect("a fix");
    assert!(read.from_simulator);
    assert!((read.latitude_deg - 47.397_742).abs() < 1e-9);
}

#[test]
fn a_stationary_vehicle_states_a_heading_and_no_track() {
    // Below the floor the velocity is noise around a parked vehicle. A track
    // drawn from it would swing wildly while the vehicle sat still.
    let read = from_sample(&truth(0.0, 0.0)).expect("a fix");
    assert_eq!(read.heading_deg, Some(0.0));
    assert_eq!(read.course_deg, None);
    assert_eq!(read.ground_speed_mps, None);
}

#[test]
fn a_moving_vehicle_states_its_track_over_the_ground() {
    // Due east at 3 m/s is 090, whatever the nose is doing.
    let read = from_sample(&truth(0.0, 3.0)).expect("a fix");
    assert_eq!(read.heading_deg, Some(0.0));
    let course = read.course_deg.expect("a course");
    assert!((course - 90.0).abs() < 1e-9, "course {course}");
    assert!((read.ground_speed_mps.expect("speed") - 3.0).abs() < 1e-6);
}

#[test]
fn an_unstated_attitude_withholds_the_heading() {
    // The mask is the authorization. Reading the quaternion anyway would turn
    // the mark by a number nobody claimed.
    let mut sample = truth(3.0, 0.0);
    sample.sim_truth.as_mut().expect("truth lane").valid_flags = 0b1000;
    let read = from_sample(&sample).expect("a fix");
    assert_eq!(read.heading_deg, None);
    assert!(read.course_deg.is_some(), "velocity is still stated");
}

#[test]
fn a_quaternion_that_is_not_one_is_not_an_attitude() {
    // An all-zero quaternion is a field nobody filled in, and it would
    // otherwise yield a confident heading of zero.
    let mut sample = truth(0.0, 0.0);
    let lane = sample.sim_truth.as_mut().expect("truth lane");
    lane.quat_w = 0.0;
    assert_eq!(from_sample(&sample).expect("a fix").heading_deg, None);
}

#[test]
fn the_estimate_lane_needs_its_status_observation() {
    // The mask on the estimate lane is meaningless without the observation
    // backing it, so its directions are withheld rather than assumed good.
    let sample = wire::TelemetrySample {
        avionics: Some(wire::AvionicsState {
            quat_w: 1.0,
            vel_n_mps: 3.0,
            valid_flags: ATTITUDE_AND_VELOCITY,
            geodetic: Some(fix()),
            geodetic_stamp: Some(stamp()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let read = from_sample(&sample).expect("a fix");
    assert!(!read.from_simulator);
    assert_eq!(
        read.heading_deg, None,
        "no status observation authorizes it"
    );
    assert_eq!(read.course_deg, None);

    // With the observation present the same sample states both.
    let mut authorized = sample;
    authorized
        .avionics
        .as_mut()
        .expect("estimate lane")
        .estimator_status_stamp = Some(stamp());
    let read = from_sample(&authorized).expect("a fix");
    assert_eq!(read.heading_deg, Some(0.0));
    assert_eq!(read.course_deg, Some(0.0));
}

#[test]
fn the_fix_is_withheld_when_the_estimate_states_no_position() {
    // The geodetic fix carries its own stamp and advances on its own, so a
    // lane with attitude but no position draws no mark at all.
    let sample = wire::TelemetrySample {
        avionics: Some(wire::AvionicsState {
            quat_w: 1.0,
            valid_flags: ATTITUDE_AND_VELOCITY,
            estimator_status_stamp: Some(stamp()),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(from_sample(&sample).is_none());
}
