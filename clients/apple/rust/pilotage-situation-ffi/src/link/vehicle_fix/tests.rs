//! What the map is allowed to draw from a telemetry sample.
//!
//! These restate the browser's rules against the same values. Nothing shared
//! enforces the agreement, so the numbers are stated outright here: a reader
//! comparing this against `clients/web/situation-ownship.js` can see in one
//! pass whether the two clients still draw the same mark.

#![allow(clippy::expect_used, clippy::panic)]

use pilotage_protocol::wire;

use super::{GroupAdvance, VehicleFix, from_sample};

/// Reads a sample the way a client that has just connected reads its first
/// one: nothing seen before, so every group states a new measurement.
fn first_sample(sample: &wire::TelemetrySample) -> Option<VehicleFix> {
    from_sample(&mut GroupAdvance::default(), sample, 0)
}

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
    assert!(first_sample(&wire::TelemetrySample::default()).is_none());
}

#[test]
fn truth_without_its_stamp_is_unconsumable() {
    // A position that cannot be shown to be this vehicle's, this boot's and
    // this moment's is not drawn, however plausible its numbers.
    let mut sample = truth(3.0, 0.0);
    sample.sim_truth.as_mut().expect("truth lane").stamp = None;
    assert!(first_sample(&sample).is_none());
}

#[test]
fn the_oracle_is_preferred_and_says_so() {
    let read = first_sample(&truth(3.0, 0.0)).expect("a fix");
    assert!(read.from_simulator);
    assert!((read.latitude_deg - 47.397_742).abs() < 1e-9);
}

#[test]
fn a_stationary_vehicle_states_a_heading_and_no_track() {
    // Below the floor the velocity is noise around a parked vehicle. A track
    // drawn from it would swing wildly while the vehicle sat still.
    let read = first_sample(&truth(0.0, 0.0)).expect("a fix");
    assert_eq!(read.heading_deg, Some(0.0));
    assert_eq!(read.course_deg, None);
    assert_eq!(read.ground_speed_mps, None);
}

#[test]
fn a_moving_vehicle_states_its_track_over_the_ground() {
    // Due east at 3 m/s is 090, whatever the nose is doing.
    let read = first_sample(&truth(0.0, 3.0)).expect("a fix");
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
    let read = first_sample(&sample).expect("a fix");
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
    assert_eq!(first_sample(&sample).expect("a fix").heading_deg, None);
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
            attitude_stamp: Some(stamp()),
            kinematics_stamp: Some(stamp()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let read = first_sample(&sample).expect("a fix");
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
    let read = first_sample(&authorized).expect("a fix");
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
    assert!(first_sample(&sample).is_none());
}

#[test]
fn an_estimator_that_calls_its_own_solution_unusable_is_believed() {
    // The clearest refusal there is. Reading the directions anyway would turn
    // the mark by a number the estimator itself disowned — and the browser
    // withholds them, so drawing them here would put the two clients on
    // different answers from one sample.
    let sample = wire::TelemetrySample {
        avionics: Some(wire::AvionicsState {
            quat_w: 1.0,
            vel_n_mps: 3.0,
            valid_flags: ATTITUDE_AND_VELOCITY,
            estimator_status_stamp: Some(stamp()),
            quality: 2,
            geodetic: Some(fix()),
            geodetic_stamp: Some(stamp()),
            attitude_stamp: Some(stamp()),
            kinematics_stamp: Some(stamp()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let read = first_sample(&sample).expect("the position still stands");
    assert_eq!(
        read.heading_deg, None,
        "an unusable solution turned the mark"
    );
    assert_eq!(read.course_deg, None, "an unusable solution drew a course");
    assert_eq!(read.ground_speed_mps, None);

    // Degraded is not unusable: the estimator is still standing behind it.
    let mut degraded = sample;
    degraded.avionics.as_mut().expect("estimate lane").quality = 1;
    let read = first_sample(&degraded).expect("a fix");
    assert_eq!(
        read.heading_deg,
        Some(0.0),
        "a degraded solution was refused"
    );
}

/// A stamp stating measurement `sequence`, so successive values read as
/// successive measurements of the same group.
fn stamp_seq(sequence: u32) -> wire::MeasurementStamp {
    wire::MeasurementStamp {
        sequence,
        ..Default::default()
    }
}

/// An estimate lane stating a position, an attitude and a velocity, each
/// group stamped separately so one can be frozen while another advances.
fn estimate(attitude: u32, kinematics: u32, geodetic: u32) -> wire::TelemetrySample {
    wire::TelemetrySample {
        avionics: Some(wire::AvionicsState {
            quat_w: 1.0,
            vel_n_mps: 3.0,
            valid_flags: ATTITUDE_AND_VELOCITY,
            geodetic: Some(fix()),
            geodetic_stamp: Some(stamp_seq(geodetic)),
            attitude_stamp: Some(stamp_seq(attitude)),
            kinematics_stamp: Some(stamp_seq(kinematics)),
            estimator_status_stamp: Some(stamp()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[test]
fn a_group_that_states_no_stamp_states_no_direction() {
    // The mask says the lane states an attitude; nothing says when it was
    // measured. A direction that cannot be shown current is not drawn — the
    // failure of a stale one is not that it fades, but that it turns the mark.
    let mut sample = estimate(1, 1, 1);
    sample
        .avionics
        .as_mut()
        .expect("estimate lane")
        .attitude_stamp = None;
    let read = first_sample(&sample).expect("the position still stands");
    assert_eq!(
        read.heading_deg, None,
        "an attitude nothing timed turned the mark"
    );
    assert_eq!(
        read.course_deg,
        Some(0.0),
        "the velocity was withheld along with it"
    );
}

#[test]
fn a_direction_stops_being_drawn_when_its_group_goes_quiet() {
    // The position keeps advancing while the attitude repeats one
    // measurement, which is what a lane looks like when its attitude source
    // stops: samples keep arriving and one group inside them stands still.
    let mut advance = GroupAdvance::default();
    let read = from_sample(&mut advance, &estimate(1, 1, 1), 0).expect("a fix");
    assert_eq!(read.heading_deg, Some(0.0));

    let read = from_sample(&mut advance, &estimate(1, 2, 2), 300).expect("a fix");
    assert_eq!(
        read.heading_deg,
        Some(0.0),
        "the bound is inclusive: at it, the group is still current"
    );

    let read = from_sample(&mut advance, &estimate(1, 3, 3), 301).expect("a fix");
    assert_eq!(
        read.heading_deg, None,
        "a group that had gone quiet went on turning the mark"
    );
    assert_eq!(
        read.course_deg,
        Some(0.0),
        "the group that kept reporting was dropped with it"
    );
    assert!(
        read.fix_advanced,
        "the position stopped being reported as new"
    );
}

#[test]
fn a_position_that_never_advances_is_not_reported_as_new() {
    // The client times staleness from when the position was last MEASURED. A
    // host relaying a frozen block delivers samples forever, and if each one
    // refreshed that clock the mark would never go stale however long the
    // vehicle had stopped reporting.
    let mut advance = GroupAdvance::default();
    assert!(
        from_sample(&mut advance, &estimate(1, 1, 1), 0)
            .expect("a fix")
            .fix_advanced
    );
    assert!(
        !from_sample(&mut advance, &estimate(1, 1, 1), 50)
            .expect("a fix")
            .fix_advanced,
        "a repeated position was called a new one"
    );
    assert!(
        from_sample(&mut advance, &estimate(1, 1, 2), 100)
            .expect("a fix")
            .fix_advanced,
        "a new position was called a repeat"
    );
}

#[test]
fn the_truth_lane_is_held_to_the_same_bound() {
    // The oracle is not exempt. A simulator that stops stepping states one
    // measurement forever, and a mark that keeps its heading through that is
    // as wrong as one on the estimate lane.
    let mut advance = GroupAdvance::default();
    assert_eq!(
        from_sample(&mut advance, &truth(3.0, 0.0), 0)
            .expect("a fix")
            .heading_deg,
        Some(0.0)
    );
    let stale = from_sample(&mut advance, &truth(3.0, 0.0), 301).expect("the position stands");
    assert_eq!(
        stale.heading_deg, None,
        "a stopped simulator went on turning the mark"
    );
    assert_eq!(stale.course_deg, None);
    assert!(!stale.fix_advanced);
}
