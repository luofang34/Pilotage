use navigate_contract::NedVelocity;

use super::*;

fn key(course_deg: f64) -> TrackKey {
    TrackKey {
        to: GeodeticPosition::new(0.7, -1.3, 2_000.0),
        course_rad: course_deg.to_radians(),
    }
}

fn at(seconds: f64) -> MonotonicNanos {
    MonotonicNanos::from_nanos((seconds * 1e9) as u64)
}

/// Steps the trim on one track every 0.1 s from `from_s` to `to_s`.
fn run(trim: &mut TrackTrim, key: TrackKey, cross_m: f64, established: bool, span: [f64; 2]) {
    let mut t = span[0];
    while t <= span[1] {
        trim.step(key, cross_m, 1e-3, established, at(t));
        t += 0.1;
    }
}

#[test]
fn a_standing_error_right_of_the_track_turns_the_course_left_once_settled() {
    let mut trim = TrackTrim::default();
    run(&mut trim, key(0.0), 10.0, true, [0.0, SETTLE_S - 0.5]);
    assert_eq!(
        trim.offset_rad, 0.0,
        "nothing integrates before the track settles"
    );
    run(&mut trim, key(0.0), 10.0, true, [SETTLE_S, SETTLE_S + 10.0]);
    // 10 m for about 10 s at 1e-3 rad per metre-second.
    assert!((trim.offset_rad + 0.1).abs() < 0.01, "{}", trim.offset_rad);
}

#[test]
fn the_trim_holds_while_not_established_and_is_bounded() {
    let mut trim = TrackTrim::default();
    run(&mut trim, key(0.0), 10.0, false, [0.0, 30.0]);
    assert_eq!(trim.offset_rad, 0.0);
    run(&mut trim, key(0.0), -1_000.0, true, [30.0, 60.0]);
    assert_eq!(trim.offset_rad, MAX_TRIM_RAD);
}

#[test]
fn a_new_track_keeps_the_trim_and_settles_again() {
    let mut trim = TrackTrim::default();
    run(&mut trim, key(0.0), 10.0, true, [0.0, 20.0]);
    let learned = trim.offset_rad;
    assert!(learned < 0.0);
    // On the next track the trim carries over and waits to settle.
    run(&mut trim, key(90.0), 10.0, true, [20.1, 25.0]);
    assert_eq!(trim.offset_rad, learned);
}

#[test]
fn the_offset_turns_a_velocity_setpoint_right_and_a_course_by_as_much() {
    let north = GuidanceSetpoint::Velocity {
        velocity: NedVelocity::new(10.0, 0.0, 0.0),
    };
    let GuidanceSetpoint::Velocity { velocity } = rotated(north, 0.1) else {
        panic!("a velocity stays a velocity");
    };
    assert!((velocity.east_mps.atan2(velocity.north_mps) - 0.1).abs() < 1e-12);
    assert!((velocity.north_mps.hypot(velocity.east_mps) - 10.0).abs() < 1e-12);
    let tracking = GuidanceSetpoint::DeviationTracking {
        lateral_m: 3.0,
        vertical_m: 0.0,
        course_rad: 1.0,
    };
    assert!(matches!(
        rotated(tracking, -0.1),
        GuidanceSetpoint::DeviationTracking { course_rad, .. } if (course_rad - 0.9).abs() < 1e-12
    ));
}

#[test]
fn tracks_are_the_same_by_their_end_and_course() {
    assert!(key(10.0).same(&key(10.2)));
    assert!(!key(10.0).same(&key(12.0)));
}
