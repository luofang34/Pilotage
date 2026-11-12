//! The pattern flight followed by a point that turns toward the path at a
//! limited rate, in the pattern frame and in still air.

use core::f64::consts::PI;

use super::super::flight::{PatternFlight, Steer};
use super::super::{HoldingEntry, HoldingLeg, STANDARD_RATE_RPS};
use super::DEG;

const STEP_S: f64 = 0.05;
const SPEED_MPS: f64 = 60.0;
/// How far the racetrack flown may reach outside the planned one, metres.
/// The point model turns with a lag, so it flies each turn a little wide.
const STEADY_MARGIN_M: f64 = 300.0;
/// How far an entry may reach outside the planned racetrack, metres: a
/// direct entry from the non-holding side rolls out on the outbound
/// course a turn radius past abeam, and its timing starts there.
fn entry_margin_m() -> f64 {
    radius_m() + STEADY_MARGIN_M
}

fn radius_m() -> f64 {
    SPEED_MPS / STANDARD_RATE_RPS
}

/// A point at a constant speed whose turn rate follows the course error
/// within the rate of a 25° bank.
struct Point {
    position: [f64; 2],
    course: f64,
}

impl Point {
    fn fly(&mut self, steer: Steer) {
        let wanted = match steer {
            Steer::Track { from, to } => {
                let course = (to[1] - from[1]).atan2(to[0] - from[0]);
                let right_m = -(self.position[0] - from[0]) * course.sin()
                    + (self.position[1] - from[1]) * course.cos();
                course - 0.5 * (right_m / 500.0).atan()
            }
            Steer::Toward { point } => {
                (point[1] - self.position[1]).atan2(point[0] - self.position[0])
            }
        };
        let error = (wanted - self.course)
            .sin()
            .atan2((wanted - self.course).cos());
        let max_rate = 9.806_65 * (25.0 * DEG).tan() / SPEED_MPS;
        self.course += (error / 3.0).clamp(-max_rate, max_rate) * STEP_S;
        self.position[0] += SPEED_MPS * self.course.cos() * STEP_S;
        self.position[1] += SPEED_MPS * self.course.sin() * STEP_S;
    }
}

/// One step of a flight: the time, the position, the course, and the leg.
struct Sample {
    t_s: f64,
    position: [f64; 2],
    course: f64,
    leg: HoldingLeg,
}

/// Flies a pattern with a leg of `leg_time_s` for `duration_s` from
/// `position` on `course`, entering at once when `at_fix`.
fn fly(
    position: [f64; 2],
    course: f64,
    leg_time_s: f64,
    at_fix: bool,
    duration_s: f64,
) -> (PatternFlight, Vec<Sample>) {
    let mut flight = PatternFlight::new(radius_m(), leg_time_s, SPEED_MPS, leg_time_s);
    let mut point = Point { position, course };
    flight.begin(position, course, 0.0, at_fix);
    let mut samples = Vec::new();
    let steps = (duration_s / STEP_S) as u32;
    for step in 0..steps {
        let t_s = f64::from(step) * STEP_S;
        let steer = flight.step(point.position, point.course, t_s);
        samples.push(Sample {
            t_s,
            position: point.position,
            course: point.course,
            leg: flight.leg(),
        });
        point.fly(steer);
    }
    (flight, samples)
}

/// The legs flown in order, with the time each started.
fn legs(samples: &[Sample]) -> Vec<(HoldingLeg, f64)> {
    let mut legs: Vec<(HoldingLeg, f64)> = Vec::new();
    for sample in samples {
        if legs.last().is_none_or(|(leg, _)| *leg != sample.leg) {
            legs.push((sample.leg, sample.t_s));
        }
    }
    legs
}

/// The time of each inbound leg that ended at the fix, seconds.
fn inbound_times(samples: &[Sample]) -> Vec<f64> {
    legs(samples)
        .windows(2)
        .filter(|pair| pair[0].0 == HoldingLeg::Inbound && pair[1].0 == HoldingLeg::FixTurn)
        .map(|pair| pair[1].1 - pair[0].1)
        .collect()
}

/// The greatest course change to the left in any pattern turn, radians
/// per step: positive means a turn against the pattern direction.
fn worst_left_turn(samples: &[Sample]) -> f64 {
    samples
        .windows(2)
        .filter(|pair| {
            matches!(pair[0].leg, HoldingLeg::FixTurn | HoldingLeg::OutboundTurn)
                && pair[0].leg == pair[1].leg
        })
        .map(|pair| {
            let change = pair[1].course - pair[0].course;
            -change.sin().atan2(change.cos())
        })
        .fold(f64::NEG_INFINITY, f64::max)
}

/// How far, metres, the samples after `after_s` reach outside the
/// racetrack's area: the extremes of `x` and `y` past the area's edges,
/// as `[behind, ahead, outside, beyond]`.
fn excursions(samples: &[Sample], leg_time_s: f64, after_s: f64) -> [f64; 4] {
    let length_m = SPEED_MPS * leg_time_s;
    let radius = radius_m();
    samples
        .iter()
        .filter(|s| s.t_s >= after_s)
        .fold([f64::NEG_INFINITY; 4], |worst, s| {
            let [x, y] = s.position;
            [
                worst[0].max(-length_m - radius - x),
                worst[1].max(x - radius),
                worst[2].max(-y),
                worst[3].max(y - 2.0 * radius),
            ]
        })
}

/// Whether every sample after `after_s` lies in the racetrack's area grown
/// by `margin_m`.
fn inside(samples: &[Sample], leg_time_s: f64, after_s: f64, margin_m: f64) -> bool {
    excursions(samples, leg_time_s, after_s)
        .iter()
        .all(|reach| *reach <= margin_m)
}

/// The time the `n`th turn at the fix started, counting from one.
fn fix_turn_start(samples: &[Sample], n: usize) -> f64 {
    legs(samples)
        .iter()
        .filter(|(leg, _)| *leg == HoldingLeg::FixTurn)
        .nth(n.saturating_sub(1))
        .map_or(f64::NAN, |(_, t_s)| *t_s)
}

#[test]
fn arriving_on_the_inbound_course_enters_direct_and_flies_one_minute_legs() {
    let (flight, samples) = fly([-3_000.0, 0.0], 0.0, 60.0, false, 1_200.0);
    let flown = legs(&samples);
    let order: Vec<HoldingLeg> = flown.iter().map(|(leg, _)| *leg).take(6).collect();
    assert_eq!(
        order,
        [
            HoldingLeg::ToFix,
            HoldingLeg::Entry(HoldingEntry::Direct),
            HoldingLeg::Outbound,
            HoldingLeg::OutboundTurn,
            HoldingLeg::Inbound,
            HoldingLeg::FixTurn,
        ]
    );
    let inbound = inbound_times(&samples);
    assert!(inbound.len() >= 3, "{inbound:?}");
    for time in inbound.iter().skip(1) {
        assert!((time - 60.0).abs() < 3.0, "inbound legs {inbound:?}");
    }
    assert!(flight.circuits() >= 3);
    assert!(
        worst_left_turn(&samples) < 0.1 * DEG,
        "a pattern turn went left"
    );
    let settled_s = fix_turn_start(&samples, 2);
    assert!(
        inside(&samples, 60.0, settled_s, STEADY_MARGIN_M),
        "the vehicle left the pattern: {:?}",
        excursions(&samples, 60.0, settled_s)
    );
    assert!(
        inside(&samples, 60.0, 60.0, entry_margin_m()),
        "the entry left the pattern: {:?}",
        excursions(&samples, 60.0, 60.0)
    );
}

#[test]
fn above_14000_ft_the_inbound_leg_lasts_ninety_seconds() {
    let (_, samples) = fly([-3_000.0, 0.0], 0.0, 90.0, false, 1_500.0);
    let inbound = inbound_times(&samples);
    assert!(inbound.len() >= 3, "{inbound:?}");
    for time in inbound.iter().skip(1) {
        assert!((time - 90.0).abs() < 3.0, "inbound legs {inbound:?}");
    }
    let settled_s = fix_turn_start(&samples, 2);
    assert!(
        inside(&samples, 90.0, settled_s, STEADY_MARGIN_M),
        "{:?}",
        excursions(&samples, 90.0, settled_s)
    );
}

/// Enters at the fix on `course_deg` and returns the entry flown and
/// whether the pattern was then flown inside its area.
fn enter_on(course_deg: f64) -> (Option<HoldingEntry>, Vec<Sample>) {
    let (flight, samples) = fly([0.0, 0.0], course_deg * DEG, 60.0, true, 1_200.0);
    assert!(flight.circuits() >= 3, "{} circuits", flight.circuits());
    let first = samples.first().map(|sample| sample.leg);
    let entry = match first {
        Some(HoldingLeg::Entry(entry)) => Some(entry),
        _ => None,
    };
    (entry, samples)
}

#[test]
fn each_entry_joins_the_pattern_and_stays_in_it() {
    for (course, entry) in [
        (0.0, HoldingEntry::Direct),
        (90.0, HoldingEntry::Direct),
        (-60.0, HoldingEntry::Direct),
        (230.0, HoldingEntry::Parallel),
        (150.0, HoldingEntry::Teardrop),
    ] {
        let (flown, samples) = enter_on(course);
        assert_eq!(flown, Some(entry), "course {course}°");
        let fix_turn = fix_turn_start(&samples, 1);
        assert!(
            fix_turn < 360.0,
            "course {course}°: the fix at {fix_turn} s"
        );
        assert!(
            inside(&samples, 60.0, 0.0, entry_margin_m()),
            "course {course}°: the entry left the pattern by {:?}",
            excursions(&samples, 60.0, 0.0)
        );
        // One circuit after the entry, the vehicle flies the racetrack.
        let settled_s = fix_turn_start(&samples, 2);
        assert!(
            inside(&samples, 60.0, settled_s, STEADY_MARGIN_M),
            "course {course}°: outside the pattern by {:?}",
            excursions(&samples, 60.0, settled_s)
        );
        assert!(worst_left_turn(&samples) < 0.1 * DEG, "course {course}°");
    }
}

#[test]
fn the_parallel_entry_turns_left_and_the_teardrop_right_back_to_the_inbound_track() {
    let turn_of = |samples: &[Sample]| {
        let entry_turn: Vec<&Sample> = samples
            .iter()
            .filter(|s| matches!(s.leg, HoldingLeg::Entry(_)) && s.t_s > 62.0)
            .collect();
        let start = entry_turn.first().map_or(0.0, |s| s.course);
        let end = entry_turn.last().map_or(0.0, |s| s.course);
        end - start
    };
    let (_, parallel) = enter_on(230.0);
    let (_, teardrop) = enter_on(150.0);
    assert!(
        turn_of(&parallel) < -180.0 * DEG,
        "{}",
        turn_of(&parallel) / DEG
    );
    assert!(
        turn_of(&teardrop) > 180.0 * DEG,
        "{}",
        turn_of(&teardrop) / DEG
    );
}

#[test]
fn a_gap_in_the_steps_enters_again_from_where_the_vehicle_is() {
    let mut flight = PatternFlight::new(radius_m(), 60.0, SPEED_MPS, 60.0);
    flight.begin([0.0, 0.0], 0.0, 0.0, true);
    assert_eq!(flight.leg(), HoldingLeg::Entry(HoldingEntry::Direct));
    // Five seconds later the vehicle is 4 km away heading away from the fix.
    let steer = flight.step([-4_000.0, 1_000.0], PI, 5.0);
    assert_eq!(flight.leg(), HoldingLeg::ToFix);
    assert_eq!(
        steer,
        Steer::Track {
            from: [-4_000.0, 1_000.0],
            to: [0.0, 0.0]
        }
    );
}

#[test]
fn a_vehicle_flown_far_out_of_the_pattern_enters_again() {
    let mut flight = PatternFlight::new(radius_m(), 60.0, SPEED_MPS, 60.0);
    flight.begin([0.0, 0.0], 0.0, 0.0, true);
    // With no gap in the steps, the vehicle is 10 km to the side.
    flight.step([0.0, -10_000.0], 0.0, 0.05);
    assert_eq!(flight.leg(), HoldingLeg::ToFix);
}

#[test]
fn the_outbound_timer_starts_within_a_leg_time_even_never_abeam() {
    let radius = radius_m();
    let mut flight = PatternFlight::new(radius, 60.0, SPEED_MPS, 60.0);
    // Entered 100 m past the fix: the turn ends 100 m ahead of abeam.
    flight.begin([100.0, 0.0], 0.0, 0.0, true);
    flight.step([100.0, 2.0 * radius], PI, 0.1);
    assert_eq!(flight.leg(), HoldingLeg::Outbound);
    // The aircraft stays ahead of abeam: the timer starts after one leg
    // time, and the outbound leg ends one outbound time later.
    let mut t = 0.2;
    while t < 119.0 {
        flight.step([100.0, 2.0 * radius], PI, t);
        assert_eq!(flight.leg(), HoldingLeg::Outbound, "{t} s");
        t += 0.1;
    }
    while t < 120.5 {
        flight.step([100.0, 2.0 * radius], PI, t);
        t += 0.1;
    }
    assert_eq!(flight.leg(), HoldingLeg::OutboundTurn);
}

#[test]
fn a_part_of_the_pattern_that_does_not_end_is_entered_again() {
    let radius = radius_m();
    let mut flight = PatternFlight::new(radius, 60.0, SPEED_MPS, 60.0);
    flight.begin([0.0, 0.0], 0.0, 0.0, true);
    assert_eq!(flight.leg(), HoldingLeg::Entry(HoldingEntry::Direct));
    // The aircraft never moves round the turn: after five leg times the
    // pattern starts again from where it is.
    let mut t = 0.1;
    while t < 299.0 {
        flight.step([0.0, 0.0], 0.0, t);
        t += 0.1;
    }
    assert_eq!(flight.entries(), 1);
    flight.step([0.0, 0.0], 0.0, 301.0);
    assert_eq!(flight.entries(), 2, "{:?}", flight.leg());
}
