//! What the mark points at, and when it stops pointing.

#![allow(clippy::expect_used, clippy::panic)]

use super::*;

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
    let mut advance = MarkMemory::default();
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
    let mut advance = MarkMemory::default();
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
    let mut advance = MarkMemory::default();
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

#[test]
fn a_course_already_drawn_is_held_through_the_band() {
    // A vehicle holding station reports a metre or two per second of drift.
    // Judged against one threshold, it crosses back and forth and the leader
    // flickers on and off at the telemetry rate, which reads as a vehicle
    // darting about rather than one sitting still.
    let course = |memory: &mut MarkMemory, speed: f32, sequence: u32, now_ms: u64| {
        from_sample(memory, &moving_at(speed, sequence), now_ms)
            .expect("a fix")
            .course_deg
            .is_some()
    };
    let mut memory = MarkMemory::default();

    assert!(
        course(&mut memory, 3.0, 1, 0),
        "a moving vehicle states a course"
    );
    assert!(
        course(&mut memory, 0.4, 2, 10),
        "a course already drawn was dropped inside the band"
    );
    assert!(
        !course(&mut memory, 0.3, 3, 20),
        "a course survived below the speed that releases it"
    );
    assert!(
        !course(&mut memory, 0.4, 4, 30),
        "a course came back without passing the floor again"
    );
    assert!(
        course(&mut memory, 0.6, 5, 40),
        "a speed above the floor was refused a course"
    );
}

#[test]
fn the_estimate_lanes_course_stops_when_its_velocity_group_goes_quiet() {
    // The one gate of the four with nothing behind it — and the estimate lane
    // is the only lane a physical vehicle flies. Its three siblings each fail
    // a test when removed; replacing this one with the raw velocity passed the
    // whole suite.
    //
    // Attitude and position keep advancing here so that only the velocity
    // group is quiet: a test where everything stopped would pass on the fix
    // gate alone and prove nothing about this one.
    let mut memory = MarkMemory::default();
    let read = from_sample(&mut memory, &estimate(1, 1, 1), 0).expect("a fix");
    assert_eq!(read.course_deg, Some(0.0));

    let read = from_sample(&mut memory, &estimate(2, 1, 2), 300).expect("a fix");
    assert_eq!(
        read.course_deg,
        Some(0.0),
        "the bound is inclusive: at it, the group is still current"
    );

    let read = from_sample(&mut memory, &estimate(3, 1, 3), 301).expect("a fix");
    assert_eq!(
        read.course_deg, None,
        "a velocity group that had gone quiet went on drawing a course"
    );
    assert_eq!(read.ground_speed_mps, None);
    assert_eq!(
        read.heading_deg,
        Some(0.0),
        "the attitude group was dropped along with it"
    );
}
