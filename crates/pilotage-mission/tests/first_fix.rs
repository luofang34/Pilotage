#![cfg(test)]

//! Where a fixed wing starts against a plan's first fix, and what it
//! flies after: wherever it starts and however far its climb carries it,
//! it sequences every fix in turn, never turns back for one it is past,
//! and holds at the last.
//!
//! The aircraft climbs before it follows the plan. A climb at cruise
//! carries it several kilometres, past the second fix here, so a first
//! fix judged only before the climb, or only up to the second fix, would
//! be turned back for.

use navigate_contract::{ClockDomainId, GeodeticPosition, MonotonicNanos};
use navigate_geodesy::NedOffset;
use navigate_geodesy::wgs84::tangent_plane;
use navigate_route::SequenceReason;
use pilotage_mission::{
    ExecutionTarget, MissionConfig, MissionEngine, MissionEvent, OwnshipSample, OwnshipSource,
    PlannedWaypoint, TruthRole, WaypointPlan, true_airspeed_for,
};
use pilotage_protocol::{ControlAction, ControlIntent, VehicleClass};

const CLOCK: ClockDomainId = ClockDomainId::new(7);
const STEP_NANOS: u64 = 50_000_000;
const STEP_S: f64 = 0.05;
const ALTITUDE_M: f64 = 2_000.0;
/// The climb before the plan is followed: about 3 km at cruise.
const CLIMB_M: f64 = 300.0;
const CRUISE_IAS_MPS: f64 = 55.0;
const MAX_IAS_MPS: f64 = 65.0;
const GRAVITY_MPS2: f64 = 9.806_65;
/// Aviate C172 `route_bank_angle` and `course_time_constant`.
const BANK_RAD: f64 = 0.44;
const COURSE_TIME_CONSTANT_S: f64 = 3.0;
/// North 2 km, then east 3 km.
const POINTS: [[f64; 2]; 3] = [[0.0, 0.0], [2_000.0, 0.0], [2_000.0, 3_000.0]];

fn anchor() -> GeodeticPosition {
    GeodeticPosition::new(40.0_f64.to_radians(), -75.0_f64.to_radians(), ALTITUDE_M)
}

fn config(climb_m: f64) -> MissionConfig {
    let mut config = MissionConfig::new(String::new(), anchor(), CLOCK);
    config.ownship_source = OwnshipSource::SimulationTruth;
    config.execution_target = ExecutionTarget::Simulator;
    config.vehicle_class = VehicleClass::FixedWing;
    config.cruise_height_m = climb_m;
    config.climb_rate_mps = 5.0;
    config.cruise_mps = CRUISE_IAS_MPS;
    config.limits.max_horizontal_mps = MAX_IAS_MPS;
    config.limits.max_vertical_mps = 5.0;
    config.limits.max_yaw_rate_rps = 0.2;
    config.route_bank_deg = 22.0;
    config
}

fn plan(points: &[[f64; 2]], climb_m: f64) -> WaypointPlan {
    let plane = tangent_plane(anchor()).expect("anchor is plausible");
    WaypointPlan::new(
        points
            .iter()
            .map(|[north, east]| {
                let p = plane.from_ned(&NedOffset::new(*north, *east, 0.0));
                PlannedWaypoint::new(
                    p.latitude_rad.to_degrees(),
                    p.longitude_rad.to_degrees(),
                    ALTITUDE_M + climb_m,
                )
            })
            .collect(),
    )
}

/// A fixed wing that flies the engine's velocity intents as a fixed
/// wing's flight controller reads them: the horizontal direction is a
/// course, its magnitude an indicated airspeed, no direction holds the
/// course; the course turns toward the demand at the route bank at most.
struct Aircraft {
    ned: [f64; 3],
    course_rad: f64,
    indicated_mps: f64,
}

impl Aircraft {
    fn velocity(&self, climb_mps: f64) -> [f64; 3] {
        let speed = true_airspeed_for(self.indicated_mps, ALTITUDE_M - self.ned[2]);
        [
            speed * self.course_rad.cos(),
            speed * self.course_rad.sin(),
            -climb_mps,
        ]
    }

    fn step(&mut self, intent: Option<&ControlIntent>) -> f64 {
        let mut climb_mps = 0.0;
        if let Some(ControlIntent::Velocity(v)) = intent {
            let (sin, cos) = self.course_rad.sin_cos();
            let (vx, vy) = (f64::from(v.vx), f64::from(v.vy));
            let [north, east] = [vx * cos - vy * sin, vx * sin + vy * cos];
            if north.hypot(east) > 0.0 {
                let error = east.atan2(north) - self.course_rad;
                let error = error.sin().atan2(error.cos());
                let speed = true_airspeed_for(self.indicated_mps, ALTITUDE_M - self.ned[2]);
                let max_rate = GRAVITY_MPS2 * BANK_RAD.tan() / speed;
                let rate = (error / COURSE_TIME_CONSTANT_S).clamp(-max_rate, max_rate);
                self.course_rad += rate * STEP_S;
                self.indicated_mps = north.hypot(east).min(MAX_IAS_MPS);
            }
            climb_mps = -f64::from(v.vz);
        }
        let velocity = self.velocity(climb_mps);
        for (position, rate) in self.ned.iter_mut().zip(velocity) {
            *position += rate * STEP_S;
        }
        climb_mps
    }
}

struct Flight {
    /// The north-east position at each tick.
    track: Vec<[f64; 2]>,
    /// Each leg advance, in order: the leg made active and why.
    advances: Vec<(usize, SequenceReason)>,
    /// The tick the holding pattern at the last fix started, if it did.
    held_at: Option<usize>,
}

/// Where and how one flight starts.
struct Start {
    /// The plan's waypoints, north-east metres from the anchor.
    points: [[f64; 2]; 3],
    /// The aircraft's north-east position; it flies north, level at the
    /// anchor's altitude.
    at: [f64; 2],
    /// The climb before the plan is followed, metres.
    climb_m: f64,
    /// The north position from which the engine gets a solution; none
    /// before, as a navigator that has not yet started.
    solved_from_north_m: f64,
}

impl Start {
    /// From `at` on [`POINTS`], climbing [`CLIMB_M`], solved at once.
    const fn at(at: [f64; 2]) -> Self {
        Self {
            points: POINTS,
            at,
            climb_m: CLIMB_M,
            solved_from_north_m: f64::NEG_INFINITY,
        }
    }
}

/// Flies the plan from `start` for at most `ticks`.
fn fly_from(start: &Start, ticks: usize) -> Flight {
    let (mut engine, _) =
        MissionEngine::from_waypoints(&plan(&start.points, start.climb_m), config(start.climb_m))
            .expect("builds");
    let mut aircraft = Aircraft {
        ned: [start.at[0], start.at[1], 0.0],
        course_rad: 0.0,
        indicated_mps: CRUISE_IAS_MPS,
    };
    let mut flight = Flight {
        track: Vec::new(),
        advances: Vec::new(),
        held_at: None,
    };
    let mut climb_mps = 0.0;
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    for tick in 0..ticks {
        now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
        let sample = OwnshipSample {
            ned: aircraft.ned,
            ned_velocity: aircraft.velocity(climb_mps),
            yaw_rad: Some(aircraft.course_rad),
            role: TruthRole::SimulationTruth,
            acquired_at: now,
            sequence: u32::try_from(tick).expect("a short flight"),
        };
        if aircraft.ned[0] >= start.solved_from_north_m {
            engine.on_ownship(&sample, now);
        }
        let output = engine.tick(now);
        if let Some(action) = output.action {
            assert!(matches!(action.action, ControlAction::Arm));
            engine.on_action_result(action.action_id, true);
        }
        for event in &output.events {
            match event {
                MissionEvent::LegAdvanced { to_index, reason } => {
                    flight.advances.push((*to_index, *reason));
                }
                MissionEvent::HoldingPatternStarted { .. } => {
                    flight.held_at.get_or_insert(tick);
                }
                _ => {}
            }
        }
        flight.track.push([aircraft.ned[0], aircraft.ned[1]]);
        climb_mps = aircraft.step(output.intent.as_ref());
        if flight.held_at.is_some_and(|held| tick > held + 200) {
            break;
        }
    }
    flight
}

/// The closest the aircraft came to the first fix after it was first
/// `away_m` from it, metres; infinite if it never got that far.
fn closest_return_m(flight: &Flight, away_m: f64) -> f64 {
    let distance = |p: &[f64; 2]| p[0].hypot(p[1]);
    flight
        .track
        .iter()
        .skip_while(|p| distance(p) < away_m)
        .map(distance)
        .fold(f64::INFINITY, f64::min)
}

/// Every leg advanced to once and in order, and the hold reached.
fn assert_flies_through(flight: &Flight) {
    let legs: Vec<usize> = flight.advances.iter().map(|(leg, _)| *leg).collect();
    assert_eq!(
        legs,
        [1, 2],
        "each fix sequenced once, in order: {:?}",
        flight.advances
    );
    assert!(
        flight.held_at.is_some(),
        "the hold at the last fix is reached: {:?}",
        flight.advances
    );
}

#[test]
fn starting_over_the_first_fix_it_is_sequenced_and_never_flown_back_to() {
    let flight = fly_from(&Start::at([0.0, 0.0]), 20_000);
    assert_flies_through(&flight);
    assert!(
        matches!(
            flight.advances.first(),
            Some((1, SequenceReason::Passed { .. }))
        ),
        "the first fix the aircraft starts over is passed: {:?}",
        flight.advances
    );
    let back = closest_return_m(&flight, 1_500.0);
    assert!(
        back > 1_000.0,
        "the aircraft came back to {back:.0} m of the first fix"
    );
}

#[test]
fn starting_2_km_before_the_first_fix_it_is_flown_to_and_captured() {
    // A short climb, so the aircraft is still short of the fix when it
    // follows the plan: a climb that carries it past is the case above.
    let flight = fly_from(
        &Start {
            climb_m: 30.0,
            ..Start::at([-2_000.0, 0.0])
        },
        20_000,
    );
    assert_flies_through(&flight);
    assert!(
        matches!(flight.advances.first(), Some((1, reason)) if !matches!(reason, SequenceReason::Passed { .. })),
        "the first fix ahead is captured: {:?}",
        flight.advances
    );
}

#[test]
fn starting_300_m_past_the_first_fix_it_is_passed_at_the_start() {
    let flight = fly_from(&Start::at([300.0, 0.0]), 20_000);
    assert_flies_through(&flight);
    assert!(
        matches!(
            flight.advances.first(),
            Some((1, SequenceReason::Passed { cross_track_m })) if cross_track_m.abs() < 1.0
        ),
        "the first fix behind is passed: {:?}",
        flight.advances
    );
    let back = closest_return_m(&flight, 1_000.0);
    assert!(
        back > 800.0,
        "the aircraft came back to {back:.0} m of the first fix"
    );
}

#[test]
fn a_fix_missed_1_km_to_the_side_is_sequenced_on_passing_abeam() {
    // Abeam the first fix 1 km to its east: the first leg is joined from
    // there, and its 2 km cannot close a kilometre at the intercept a
    // fixed wing flies, so the second fix is passed wide.
    let flight = fly_from(&Start::at([0.0, 1_000.0]), 20_000);
    assert_flies_through(&flight);
    let second = flight.advances.get(1);
    assert!(
        matches!(second, Some((2, SequenceReason::Passed { cross_track_m })) if cross_track_m.abs() > 100.0),
        "the second fix is passed abeam, wide of it: {:?}",
        flight.advances
    );
    // No turn back toward the missed fix: from abeam it to the hold, the
    // aircraft only draws away from it.
    let [fix_north, fix_east] = POINTS[1];
    let from_fix = |p: &[f64; 2]| (p[0] - fix_north).hypot(p[1] - fix_east);
    let after_abeam: Vec<f64> = flight
        .track
        .get(..flight.held_at.unwrap_or(0))
        .unwrap_or_default()
        .iter()
        .skip_while(|p| p[0] < fix_north)
        .map(from_fix)
        .collect();
    let closer = after_abeam
        .windows(2)
        .map(|pair| pair[0] - pair[1])
        .fold(0.0_f64, f64::max);
    assert!(
        closer < 0.5,
        "the aircraft closed on the missed fix by {closer:.2} m in one tick"
    );
}

#[test]
fn a_mission_that_first_follows_2_km_past_the_first_fix_flies_on_from_there() {
    // The navigator first solves 2 km past the first fix, as a visual set
    // that reports only after the aircraft left the fix it started on;
    // the climb then carries the aircraft past the second fix too.
    let flight = fly_from(
        &Start {
            points: [[0.0, 0.0], [4_000.0, 0.0], [4_000.0, 3_000.0]],
            solved_from_north_m: 2_000.0,
            ..Start::at([0.0, 0.0])
        },
        20_000,
    );
    assert_flies_through(&flight);
    assert!(
        matches!(
            flight.advances.first(),
            Some((1, SequenceReason::Passed { .. }))
        ),
        "the first leg is made active, the second fix its TO fix: {:?}",
        flight.advances
    );
    let back = closest_return_m(&flight, 2_000.0);
    assert!(
        back >= 1_999.0,
        "the aircraft came back to {back:.0} m of the first fix"
    );
}

#[test]
fn starting_5_km_short_and_600_m_aside_it_is_passed_on_crossing_the_perpendicular() {
    // The live case, in this plan's geometry: a STAR link (KAUS BLEWE5
    // from WINDU) whose aircraft starts 5 km short of the first fix and
    // 600 m to its side, and first follows the plan only once its initial
    // heading has carried it across the first fix's perpendicular, and
    // then climbs past the second fix as well.
    let flight = fly_from(
        &Start {
            solved_from_north_m: 100.0,
            ..Start::at([-5_000.0, 600.0])
        },
        20_000,
    );
    assert_flies_through(&flight);
    assert!(
        matches!(
            flight.advances.first(),
            Some((1, SequenceReason::Passed { cross_track_m })) if (cross_track_m - 600.0).abs() < 50.0
        ),
        "the first fix is passed on crossing its perpendicular, 600 m aside: {:?}",
        flight.advances
    );
    let first_back = flight
        .track
        .iter()
        .skip_while(|p| p[0] < 100.0)
        .map(|p| p[0])
        .fold(f64::INFINITY, f64::min);
    assert!(
        first_back > 0.0,
        "the aircraft went back south of the first fix's perpendicular: {first_back:.0} m"
    );
}
