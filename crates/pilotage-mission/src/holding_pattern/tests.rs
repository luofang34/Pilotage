use core::f64::consts::{FRAC_PI_2, PI};

use navigate_contract::GeodeticPosition;

use super::*;

const DEG: f64 = PI / 180.0;

fn right_entry(course_deg: f64) -> HoldingEntry {
    entry_for(0.0, HoldingTurn::Right, course_deg * DEG)
}

#[test]
fn the_course_on_the_inbound_track_enters_direct() {
    assert_eq!(right_entry(0.0), HoldingEntry::Direct);
    assert_eq!(
        entry_for(270.0 * DEG, HoldingTurn::Left, 270.0 * DEG),
        HoldingEntry::Direct
    );
}

#[test]
fn right_turn_sectors_split_at_the_outbound_course_and_the_70_degree_line() {
    // Outbound is 180°: up to 110° right of it is parallel, up to 70° left
    // of it is teardrop, and the 180° between is direct.
    for (course, entry) in [
        (180.5, HoldingEntry::Parallel),
        (289.5, HoldingEntry::Parallel),
        (290.5, HoldingEntry::Direct),
        (109.5, HoldingEntry::Direct),
        (110.5, HoldingEntry::Teardrop),
        (179.5, HoldingEntry::Teardrop),
        (180.0, HoldingEntry::Teardrop),
        (90.0, HoldingEntry::Direct),
        (-60.0, HoldingEntry::Direct),
    ] {
        assert_eq!(right_entry(course), entry, "course {course}°");
    }
}

#[test]
fn left_turn_sectors_mirror_the_right_turn_ones() {
    for offset in [0.5, 30.0, 69.5, 70.5, 109.5, 110.5, 150.0] {
        let right = right_entry(180.0 + offset);
        let left = entry_for(0.0, HoldingTurn::Left, (180.0 - offset) * DEG);
        assert_eq!(left, right, "offset {offset}°");
    }
}

#[test]
fn sectors_turn_with_the_inbound_course() {
    let inbound = 237.0;
    for course in [0.0, 45.0, 100.0, 150.0, 181.0, 250.0, 300.0, 359.0] {
        assert_eq!(
            entry_for(inbound * DEG, HoldingTurn::Right, (inbound + course) * DEG),
            right_entry(course),
            "course {course}° from the inbound course"
        );
    }
}

#[test]
fn the_leg_time_is_one_minute_up_to_14000_ft_and_a_minute_and_a_half_above() {
    assert_eq!(standard_leg_time_s(0.0), 60.0);
    assert_eq!(standard_leg_time_s(4_267.0), 60.0);
    assert_eq!(standard_leg_time_s(4_268.0), 90.0);
    assert_eq!(standard_leg_time_s(10_000.0), 90.0);
}

#[test]
fn the_holding_speed_limits_follow_the_icao_table() {
    let knots = |altitude_m: f64| holding_speed_limit_mps(altitude_m) * 3_600.0 / 1_852.0;
    assert!((knots(1_000.0) - 230.0).abs() < 1e-9);
    assert!((knots(5_000.0) - 240.0).abs() < 1e-9);
    assert!((knots(9_000.0) - 265.0).abs() < 1e-9);
}

#[test]
fn turns_are_at_the_standard_rate_unless_the_route_bank_is_smaller_or_the_wind_needs_more() {
    // 60 m/s at 3°/s: a radius of v / ω and about 18° of bank.
    let standard_m = 60.0 / STANDARD_RATE_RPS;
    assert!((holding_turn_radius_m(60.0, 25.0, 0.0) - standard_m).abs() < 1e-6);
    assert!(standard_rate_bank_deg(60.0) < 25.0);
    // A route bank under the standard-rate bank turns wider.
    let shallow_m = crate::turn_radius_for(60.0, 10.0, 0.0);
    assert!((holding_turn_radius_m(60.0, 10.0, 0.0) - shallow_m).abs() < 1e-6);
    // 15 m/s of tailwind needs the route bank over the ground at 75 m/s.
    let windy_m = crate::turn_radius_for(75.0, 22.0, 0.0);
    assert!(windy_m > standard_m);
    assert!((holding_turn_radius_m(60.0, 22.0, 15.0) - windy_m).abs() < 1e-6);
}

#[test]
fn the_frame_puts_the_holding_side_at_positive_y_for_either_turn() {
    let fix = GeodeticPosition::new(40.0 * DEG, -75.0 * DEG, 1_000.0);
    let plane = crate::body::tangent_plane(fix).unwrap();
    let south = plane.from_ned(&NedOffset::new(-1_000.0, 0.0, 0.0));
    let north = plane.from_ned(&NedOffset::new(1_000.0, 0.0, 0.0));
    let west = plane.from_ned(&NedOffset::new(0.0, -1_000.0, 0.0));
    let pattern = |turn| HoldingPattern {
        fix_ident: "FIX".to_owned(),
        fix,
        inbound_course_rad: FRAC_PI_2,
        turn,
        leg_time_s: 60.0,
        turn_radius_m: 1_000.0,
        leg_length_m: 3_000.0,
    };
    let right = PatternFrame::new(&pattern(HoldingTurn::Right)).unwrap();
    let left = PatternFrame::new(&pattern(HoldingTurn::Left)).unwrap();
    let close = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]) < 0.01;
    assert!(close(right.point(&south), [0.0, 1_000.0]));
    assert!(close(left.point(&north), [0.0, 1_000.0]));
    assert!(close(right.point(&west), [-1_000.0, 0.0]));
    assert!((right.course(PI) - FRAC_PI_2).abs() < 1e-12);
    assert!((left.course(0.0) - FRAC_PI_2).abs() < 1e-12);
    // South of an eastbound course is right of it, for either turn.
    assert!((right.right_of_inbound_m(&south) - 1_000.0).abs() < 0.01);
    assert!((left.right_of_inbound_m(&south) - 1_000.0).abs() < 0.01);
    let back = left.position([-1_000.0, 1_000.0]);
    assert!(close(left.point(&back), [-1_000.0, 1_000.0]));
    assert_eq!(back.altitude_m, 1_000.0);
}

mod flown;
