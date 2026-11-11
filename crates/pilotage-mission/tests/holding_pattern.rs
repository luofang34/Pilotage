#![cfg(test)]

//! A fixed wing with no instruction after its last waypoint flies the
//! standard holding pattern there, closed-loop with Aviate's C172 lateral
//! law: on the onboard navigator in still air and in a steady wind, and on
//! simulation truth in still air. It is commanded on every tick, it stays
//! in the pattern's area for at least three circuits, it turns right, and
//! a new plan takes it out of the pattern.

#[path = "holding_pattern/aircraft.rs"]
mod aircraft;

use aircraft::{ALTITUDE_M, Aircraft, CLOCK, CRUISE_IAS_MPS, STEP_NANOS, STEP_S, anchor};
use navigate_contract::MonotonicNanos;
use navigate_geodesy::NedOffset;
use navigate_geodesy::wgs84::tangent_plane;
use pilotage_mission::{
    ExecutionTarget, HoldingEntry, HoldingLeg, HoldingPattern, HoldingTurn, MissionConfig,
    MissionEngine, MissionEvent, MissionState, OwnshipSample, OwnshipSource, PlannedWaypoint,
    TruthRole, WaypointPlan, true_airspeed_for,
};
use pilotage_protocol::{ControlAction, VehicleClass};

/// The last waypoint, 6 km north of the anchor, reached northbound.
const FIX_NED: [f64; 2] = [6_000.0, 0.0];

/// Where the engine's solution comes from.
#[derive(Clone, Copy, PartialEq)]
enum Feed {
    Navigator,
    Truth,
}

fn config(feed: Feed) -> MissionConfig {
    let mut config = MissionConfig::new(String::new(), anchor(), CLOCK);
    config.ownship_source = match feed {
        Feed::Navigator => OwnshipSource::onboard_navigation(),
        Feed::Truth => OwnshipSource::SimulationTruth,
    };
    config.execution_target = ExecutionTarget::Simulator;
    config.vehicle_class = VehicleClass::FixedWing;
    config.cruise_height_m = 0.0;
    config.cruise_mps = CRUISE_IAS_MPS;
    config.limits.max_horizontal_mps = 65.0;
    config.limits.max_vertical_mps = 5.0;
    config.limits.max_yaw_rate_rps = 0.2;
    config.route_bank_deg = 22.0;
    config
}

/// A plan through anchor-plane NED `points` at the anchor's altitude.
fn plan_through(points: &[[f64; 2]]) -> WaypointPlan {
    let plane = tangent_plane(anchor()).expect("anchor is plausible");
    WaypointPlan::new(
        points
            .iter()
            .map(|[north, east]| {
                let p = plane.from_ned(&NedOffset::new(*north, *east, 0.0));
                PlannedWaypoint::new(
                    p.latitude_rad.to_degrees(),
                    p.longitude_rad.to_degrees(),
                    ALTITUDE_M,
                )
            })
            .collect(),
    )
}

/// One tick of a flight.
struct Sample {
    t_s: f64,
    ned: [f64; 3],
    heading_rad: f64,
    state: MissionState,
    commanded: bool,
    leg: Option<HoldingLeg>,
    circuits: u32,
    outbound_time_s: f64,
}

/// A closed-loop flight of one engine.
struct Run {
    engine: MissionEngine,
    aircraft: Aircraft,
    feed: Feed,
    now: MonotonicNanos,
    sequence: u32,
    samples: Vec<Sample>,
    events: Vec<MissionEvent>,
}

impl Run {
    fn new(feed: Feed, wind_mps: [f64; 2]) -> Self {
        let plan = plan_through(&[[0.0, 0.0], FIX_NED]);
        let (engine, _) = MissionEngine::from_waypoints(&plan, config(feed)).expect("builds");
        Self {
            engine,
            aircraft: Aircraft::new([-1_500.0, 0.0, 0.0], 0.0, wind_mps),
            feed,
            now: MonotonicNanos::from_nanos(1_000_000_000),
            sequence: 0,
            samples: Vec::new(),
            events: Vec::new(),
        }
    }

    /// One tick, with a solution fed only when `solved`.
    fn step(&mut self, solved: bool) {
        self.now = MonotonicNanos::from_nanos(self.now.as_nanos() + STEP_NANOS);
        self.sequence = self.sequence.wrapping_add(1);
        match (self.feed, solved) {
            (Feed::Navigator, true) => {
                let solution = self.aircraft.solution(self.now, self.sequence);
                self.engine.on_onboard_solution(&solution, self.now);
            }
            (Feed::Truth, true) => {
                let sample = OwnshipSample {
                    ned: self.aircraft.ned,
                    ned_velocity: self.aircraft.ground_velocity(),
                    yaw_rad: Some(self.aircraft.heading_rad),
                    role: TruthRole::SimulationTruth,
                    acquired_at: self.now,
                    sequence: self.sequence,
                };
                self.engine.on_ownship(&sample, self.now);
            }
            (_, false) => {}
        }
        let output = self.engine.tick(self.now);
        if let Some(action) = output.action {
            assert_eq!(action.action, ControlAction::Arm);
            self.engine.on_action_result(action.action_id, true);
        }
        self.events.extend(output.events.iter().cloned());
        let status = self.engine.holding_pattern();
        self.samples.push(Sample {
            t_s: self.samples.len() as f64 * STEP_S,
            ned: self.aircraft.ned,
            heading_rad: self.aircraft.heading_rad,
            state: output.state,
            commanded: output.intent.is_some(),
            leg: status.as_ref().map(|status| status.leg),
            circuits: status.as_ref().map_or(0, |status| status.circuits),
            outbound_time_s: status
                .as_ref()
                .map_or(f64::NAN, |status| status.outbound_time_s),
        });
        self.aircraft.fly(output.intent.as_ref());
    }

    /// Flies until the plan completes, and then for `seconds`.
    fn fly_and_hold(&mut self, seconds: f64) {
        for _ in 0..20_000 {
            self.step(true);
            if self.engine.state() == MissionState::Complete {
                break;
            }
        }
        assert_eq!(
            self.engine.state(),
            MissionState::Complete,
            "the plan completes"
        );
        for _ in 0..(seconds / STEP_S) as usize {
            self.step(true);
        }
    }

    fn pattern(&self) -> HoldingPattern {
        self.events
            .iter()
            .find_map(|event| match event {
                MissionEvent::HoldingPatternStarted { pattern } => Some(pattern.clone()),
                _ => None,
            })
            .expect("a holding pattern started")
    }

    fn entries(&self) -> Vec<HoldingEntry> {
        self.events
            .iter()
            .filter_map(|event| match event {
                MissionEvent::HoldingPatternEntered { entry } => Some(*entry),
                _ => None,
            })
            .collect()
    }
}

/// The legs flown in order, with the time each started.
fn legs(samples: &[Sample]) -> Vec<(HoldingLeg, f64)> {
    let mut legs: Vec<(HoldingLeg, f64)> = Vec::new();
    for sample in samples {
        let Some(leg) = sample.leg else { continue };
        if legs.last().is_none_or(|(last, _)| *last != leg) {
            legs.push((leg, sample.t_s));
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

/// The time the `n`th turn at the fix started, counting from one.
fn fix_turn_start(samples: &[Sample], n: usize) -> f64 {
    legs(samples)
        .iter()
        .filter(|(leg, _)| *leg == HoldingLeg::FixTurn)
        .nth(n.saturating_sub(1))
        .map_or(f64::NAN, |(_, t_s)| *t_s)
}

/// How far the samples from `after_s` on reach outside the racetrack of
/// `pattern` north of the anchor's fix, with right turns: behind the
/// outbound turn, ahead of the fix turn, off the non-holding side, and
/// past the outbound leg, metres. The outbound leg is as long as the
/// longest outbound time at the groundspeed of the wind behind.
fn excursions(
    samples: &[Sample],
    pattern: &HoldingPattern,
    wind_mps: f64,
    after_s: f64,
) -> [f64; 4] {
    let radius = pattern.turn_radius_m;
    let airspeed = true_airspeed_for(CRUISE_IAS_MPS, ALTITUDE_M);
    samples
        .iter()
        .filter(|s| s.t_s >= after_s)
        .fold([f64::NEG_INFINITY; 4], |worst, s| {
            let (x, y) = (s.ned[0] - FIX_NED[0], s.ned[1] - FIX_NED[1]);
            let length = (airspeed + wind_mps) * s.outbound_time_s;
            [
                worst[0].max(-length - radius - x),
                worst[1].max(x - radius),
                worst[2].max(-y),
                worst[3].max(y - 2.0 * radius),
            ]
        })
}

/// The largest heading change to the left in one tick of a pattern turn,
/// radians.
fn worst_left_turn(samples: &[Sample]) -> f64 {
    samples
        .windows(2)
        .filter(|pair| {
            matches!(
                pair[0].leg,
                Some(HoldingLeg::FixTurn | HoldingLeg::OutboundTurn)
            ) && pair[0].leg == pair[1].leg
        })
        .map(|pair| {
            let change = pair[1].heading_rad - pair[0].heading_rad;
            -change.sin().atan2(change.cos())
        })
        .fold(f64::NEG_INFINITY, f64::max)
}

/// The checks of a pattern flown after arriving northbound at the fix.
fn assert_holds(run: &Run, wind_mps: f64, inbound_tolerance_s: f64, summary: &str) {
    let pattern = run.pattern();
    assert_eq!(pattern.turn, HoldingTurn::Right);
    assert!(
        pattern.inbound_course_rad.sin().abs() < 1e-3 && pattern.inbound_course_rad.cos() > 0.0
    );
    assert_eq!(pattern.leg_time_s, 60.0);
    assert_eq!(
        run.entries().first(),
        Some(&HoldingEntry::Direct),
        "{summary}"
    );
    let holding: Vec<&Sample> = run
        .samples
        .iter()
        .filter(|s| s.state == MissionState::Complete)
        .collect();
    assert!(
        holding.iter().all(|s| s.commanded),
        "a tick in the pattern went without a command"
    );
    let circuits = run.samples.last().map_or(0, |s| s.circuits);
    assert!(circuits >= 3, "{circuits} circuits: {summary}");
    let inbound = inbound_times(&run.samples);
    for time in inbound.iter().skip(1) {
        assert!(
            (time - 60.0).abs() < inbound_tolerance_s,
            "inbound legs {inbound:.1?}: {summary}"
        );
    }
    let settled_s = fix_turn_start(&run.samples, 2);
    let worst = excursions(&run.samples, &pattern, wind_mps, settled_s);
    assert!(
        worst
            .iter()
            .all(|reach| *reach < 0.25 * pattern.turn_radius_m),
        "outside the pattern by {worst:.0?} m, radius {:.0} m: {summary}",
        pattern.turn_radius_m
    );
    let left = worst_left_turn(&run.samples);
    assert!(
        left < 0.2_f64.to_radians(),
        "a pattern turn went left by {left} rad: {summary}"
    );
    for s in run.samples.iter().filter(|s| s.t_s >= settled_s) {
        assert!(
            (-s.ned[2]).abs() < 5.0,
            "{} m off the holding altitude",
            -s.ned[2]
        );
    }
}

#[test]
fn on_the_navigator_in_still_air_the_pattern_is_the_racetrack() {
    let mut run = Run::new(Feed::Navigator, [0.0, 0.0]);
    run.fly_and_hold(1_200.0);
    let inbound = inbound_times(&run.samples);
    assert_holds(&run, 0.0, 3.0, &format!("inbound {inbound:.1?}"));
    let airspeed = true_airspeed_for(CRUISE_IAS_MPS, ALTITUDE_M);
    let standard_m = airspeed / pilotage_mission::holding_pattern::STANDARD_RATE_RPS;
    assert!((run.pattern().turn_radius_m - standard_m).abs() < 1e-6);
}

#[test]
fn on_the_navigator_in_a_wind_the_inbound_leg_converges_to_one_minute() {
    // 12 m/s from the north-west, and 12 m/s from the north: a headwind
    // on the inbound leg.
    for wind in [[-8.485, 8.485], [-12.0, 0.0], [0.0, -12.0]] {
        let mut run = Run::new(Feed::Navigator, wind);
        run.fly_and_hold(1_500.0);
        let inbound = inbound_times(&run.samples);
        let outbound: Vec<f64> = run
            .samples
            .iter()
            .map(|s| s.outbound_time_s)
            .filter(|t| t.is_finite())
            .collect();
        let summary = format!(
            "wind {wind:?}: inbound {inbound:.1?}, outbound {:.1?} s",
            outbound.last()
        );
        assert_holds(&run, 12.0, 5.0, &summary);
        // The radius leaves the route bank for the turn downwind.
        let airspeed = true_airspeed_for(CRUISE_IAS_MPS, ALTITUDE_M);
        let downwind_m = pilotage_mission::turn_radius_for(airspeed + 12.0, 22.0, 0.0);
        assert!(run.pattern().turn_radius_m >= downwind_m - 1.0, "{summary}");
    }
}

#[test]
fn on_simulation_truth_in_still_air_the_pattern_is_the_racetrack() {
    let mut run = Run::new(Feed::Truth, [0.0, 0.0]);
    run.fly_and_hold(1_200.0);
    let inbound = inbound_times(&run.samples);
    assert_holds(&run, 0.0, 3.0, &format!("inbound {inbound:.1?}"));
}

#[test]
fn a_lost_solution_keeps_the_commands_coming_and_the_pattern_is_entered_again() {
    let mut run = Run::new(Feed::Navigator, [-8.485, 8.485]);
    run.fly_and_hold(300.0);
    // Twenty seconds with no solution: the heading and altitude hold.
    for _ in 0..400 {
        run.step(false);
    }
    let lost = run.samples.len();
    run.fly_and_hold(900.0);
    assert!(
        run.samples
            .iter()
            .all(|s| s.state != MissionState::Complete || s.commanded)
    );
    assert!(run.entries().len() >= 2, "entries {:?}", run.entries());
    let after: Vec<u32> = run
        .samples
        .get(lost..)
        .unwrap_or_default()
        .iter()
        .map(|s| s.circuits)
        .collect();
    assert!(
        after.last().copied().unwrap_or(0) >= 2,
        "circuits after the loss {after:?}"
    );
}

#[test]
fn a_new_plan_takes_the_aircraft_out_of_the_pattern() {
    let mut run = Run::new(Feed::Navigator, [-8.485, 8.485]);
    run.fly_and_hold(400.0);
    assert!(run.engine.holding_pattern().is_some());
    // The host builds a new engine over a new plan: east of the fix, from
    // where the aircraft is.
    let here = [run.aircraft.ned[0], run.aircraft.ned[1]];
    let new_fix = [FIX_NED[0], FIX_NED[1] + 8_000.0];
    let plan = plan_through(&[here, new_fix]);
    let (engine, _) =
        MissionEngine::from_waypoints(&plan, config(Feed::Navigator)).expect("builds");
    run.engine = engine;
    run.events.clear();
    let swapped = run.samples.len();
    assert!(run.engine.holding_pattern().is_none());
    run.fly_and_hold(200.0);
    let new: &[Sample] = run.samples.get(swapped..).unwrap_or_default();
    assert!(
        new.iter()
            .filter(|s| s.state != MissionState::AwaitSolution)
            .all(|s| s.commanded)
    );
    // It left the old pattern and holds at the new fix instead.
    let pattern = run.pattern();
    let plane = tangent_plane(anchor()).expect("anchor is plausible");
    let fix = plane.to_ned(&pattern.fix);
    assert!((fix.north_m - new_fix[0]).abs() < 1.0 && (fix.east_m - new_fix[1]).abs() < 1.0);
    let farthest_east = new
        .iter()
        .map(|s| s.ned[1])
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(farthest_east > new_fix[1] - 200.0, "{farthest_east} m east");
}
