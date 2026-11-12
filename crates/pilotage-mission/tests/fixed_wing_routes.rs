#![cfg(test)]

//! A fixed wing at 2000 m flown closed-loop. It joins a plan ahead of it,
//! and it holds the indicated
//! airspeed it is commanded, so it flies faster over the ground than that
//! number, and it turns no tighter than its route bank allows. The turn
//! ahead of each fly-by waypoint must be sized for that speed, or every
//! turn swings wide of the next leg.
//!
//! The aircraft model is Aviate's fixed-wing lateral law with the C172's
//! parameters: a course error over the course time constant is a turn
//! rate, with an integrator that trims a sustained turn near the target,
//! limited to the route bank; the bank follows with the roll time constant
//! and the roll-rate limit.

use navigate_contract::{ClockDomainId, GeodeticPosition, MonotonicNanos};
use navigate_geodesy::NedOffset;
use navigate_geodesy::wgs84::tangent_plane;
use navigate_route::SequenceReason;
use pilotage_mission::{
    ExecutionTarget, MissionConfig, MissionEngine, MissionEvent, MissionState, OwnshipSample,
    OwnshipSource, PlannedWaypoint, TruthRole, WaypointPlan, WaypointTurn, true_airspeed_for,
};
use pilotage_protocol::{ControlAction, ControlIntent, VehicleClass};

const CLOCK: ClockDomainId = ClockDomainId::new(7);
const STEP_NANOS: u64 = 50_000_000;
const STEP_S: f64 = 0.05;
const ALTITUDE_M: f64 = 2_000.0;
const CRUISE_IAS_MPS: f64 = 55.0;
const MIN_IAS_MPS: f64 = 33.0;
const MAX_IAS_MPS: f64 = 65.0;
/// The bank the mission plans its turns at: under the flight controller's
/// route bank, so the aircraft has bank to spare for its roll-in. Planned
/// at the flight controller's own bank, the time the aircraft takes to
/// roll in carries every turn some 30 m past the next leg.
const ROUTE_BANK_DEG: f64 = 22.0;
/// Aviate C172 `route_bank_angle`.
const AIRCRAFT_ROUTE_BANK_RAD: f64 = 0.44;
/// Aviate C172 `course_time_constant`.
const COURSE_TIME_CONSTANT_S: f64 = 3.0;
/// Aviate C172 `course_integral_time`.
const COURSE_INTEGRAL_TIME_S: f64 = 20.0;
/// Aviate C172 `roll_time_constant`.
const ROLL_TIME_CONSTANT_S: f64 = 0.5;
/// Aviate C172 `max_roll_rate`.
const MAX_ROLL_RATE_RPS: f64 = 0.5;
const GRAVITY_MPS2: f64 = 9.806_65;
/// Each side of the square the plan flies, metres.
const SIDE_M: f64 = 4_000.0;

fn anchor() -> GeodeticPosition {
    GeodeticPosition::new(40.0_f64.to_radians(), -75.0_f64.to_radians(), ALTITUDE_M)
}

fn config() -> MissionConfig {
    let mut config = MissionConfig::new(String::new(), anchor(), CLOCK);
    config.ownship_source = OwnshipSource::SimulationTruth;
    config.execution_target = ExecutionTarget::Simulator;
    config.vehicle_class = VehicleClass::FixedWing;
    config.cruise_height_m = 0.0;
    config.cruise_mps = CRUISE_IAS_MPS;
    config.limits.max_horizontal_mps = MAX_IAS_MPS;
    config.limits.max_vertical_mps = 5.0;
    config.limits.max_yaw_rate_rps = 0.2;
    config.route_bank_deg = ROUTE_BANK_DEG;
    config
}

/// The waypoints, as anchor-plane NED offsets at the anchor's altitude:
/// the anchor, then a clockwise square north and east of it back to the
/// anchor. The first leg is flown from the aircraft's position, so its
/// end has no turn ahead of it; the three corners after it do.
fn corners() -> [[f64; 2]; 5] {
    [
        [0.0, 0.0],
        [SIDE_M, 0.0],
        [SIDE_M, SIDE_M],
        [0.0, SIDE_M],
        [0.0, 0.0],
    ]
}

/// A fly-by plan through anchor-plane NED `points` at the anchor's
/// altitude.
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

fn plan() -> WaypointPlan {
    plan_through(&corners())
}

/// A fixed wing that flies the engine's velocity intents the way Aviate's
/// fixed-wing law reads them: the horizontal direction is a course, its
/// magnitude an indicated airspeed, and no direction holds the course at
/// cruise.
struct Aircraft {
    ned: [f64; 3],
    course_rad: f64,
    bank_rad: f64,
    turn_rate_integrator: f64,
    indicated_mps: f64,
}

impl Aircraft {
    fn true_airspeed_mps(&self) -> f64 {
        true_airspeed_for(self.indicated_mps, ALTITUDE_M - self.ned[2])
    }

    fn velocity(&self) -> [f64; 3] {
        let speed = self.true_airspeed_mps();
        [
            speed * self.course_rad.cos(),
            speed * self.course_rad.sin(),
            0.0,
        ]
    }

    /// Aviate's `bank_command` for a course target.
    fn bank_command(&mut self, error: f64, rate_per_tangent: f64) -> f64 {
        let limit = AIRCRAFT_ROUTE_BANK_RAD;
        let proportional = error / COURSE_TIME_CONSTANT_S;
        let bank_of = |turn_rate: f64| (turn_rate / rate_per_tangent).atan();
        let unlimited = bank_of(proportional + self.turn_rate_integrator);
        let bank = unlimited.clamp(-limit, limit);
        let near_target = bank_of(proportional).abs() <= 0.5 * limit;
        if near_target && (unlimited - bank) * error <= 0.0 {
            let bound = limit.tan() * rate_per_tangent;
            self.turn_rate_integrator = (self.turn_rate_integrator
                + proportional / COURSE_INTEGRAL_TIME_S * STEP_S)
                .clamp(-bound, bound);
        }
        bank
    }

    fn step(&mut self, demand: Option<[f64; 2]>) {
        let mut wanted_course = self.course_rad;
        if let Some([north, east]) = demand
            && north.hypot(east) > 0.0
        {
            wanted_course = east.atan2(north);
            self.indicated_mps = north.hypot(east).clamp(MIN_IAS_MPS, MAX_IAS_MPS);
        }
        let speed = self.true_airspeed_mps();
        let rate_per_tangent = GRAVITY_MPS2 / speed;
        let error = (wanted_course - self.course_rad)
            .sin()
            .atan2((wanted_course - self.course_rad).cos());
        let wanted_bank = self.bank_command(error, rate_per_tangent);
        let roll_rate = ((wanted_bank - self.bank_rad) / ROLL_TIME_CONSTANT_S)
            .clamp(-MAX_ROLL_RATE_RPS, MAX_ROLL_RATE_RPS);
        self.bank_rad += roll_rate * STEP_S;
        self.course_rad += rate_per_tangent * self.bank_rad.tan() * STEP_S;
        let velocity = self.velocity();
        for (position, rate) in self.ned.iter_mut().zip(velocity) {
            *position += rate * STEP_S;
        }
    }
}

struct Flight {
    /// The anchor-plane position at each tick, and the leg flown then.
    track: Vec<([f64; 3], usize)>,
    /// Each leg advance: the leg and the distance to the corner it turned
    /// at, metres.
    leads: Vec<(usize, f64)>,
    /// The tick at which the plan completed.
    completed_at: Option<usize>,
    /// Each leg advance: the leg and why the waypoint before it sequenced.
    reasons: Vec<(usize, SequenceReason)>,
}

/// The flight of the square plan from 1500 m south of its first waypoint.
fn fly(engine: &mut MissionEngine, ticks_after_complete: usize) -> Flight {
    fly_from(
        engine,
        [-1_500.0, 0.0],
        &corners(),
        60_000,
        ticks_after_complete,
    )
}

/// Flies `engine` from `start`, northbound, for at most `ticks`, and for
/// `ticks_after_complete` once the plan through `points` completes.
fn fly_from(
    engine: &mut MissionEngine,
    start: [f64; 2],
    points: &[[f64; 2]],
    ticks: u32,
    ticks_after_complete: usize,
) -> Flight {
    let mut aircraft = Aircraft {
        ned: [start[0], start[1], 0.0],
        course_rad: 0.0,
        bank_rad: 0.0,
        turn_rate_integrator: 0.0,
        indicated_mps: CRUISE_IAS_MPS,
    };
    let mut flight = Flight {
        track: Vec::new(),
        leads: Vec::new(),
        completed_at: None,
        reasons: Vec::new(),
    };
    let mut leg = 0;
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    for sequence in 0..ticks {
        now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
        let sample = OwnshipSample {
            ned: aircraft.ned,
            ned_velocity: aircraft.velocity(),
            yaw_rad: Some(aircraft.course_rad),
            role: TruthRole::SimulationTruth,
            acquired_at: now,
            sequence,
        };
        engine.on_ownship(&sample, now);
        let output = engine.tick(now);
        if let Some(action) = output.action {
            assert!(matches!(action.action, ControlAction::Arm));
            engine.on_action_result(action.action_id, true);
        }
        let demand = match output.intent {
            Some(ControlIntent::Velocity(v)) => {
                let (sin, cos) = aircraft.course_rad.sin_cos();
                let (vx, vy) = (f64::from(v.vx), f64::from(v.vy));
                Some([vx * cos - vy * sin, vx * sin + vy * cos])
            }
            _ => None,
        };
        for event in &output.events {
            if let MissionEvent::LegAdvanced { to_index, reason } = event {
                leg = *to_index;
                let corner = points[leg - 1];
                let lead = (aircraft.ned[0] - corner[0]).hypot(aircraft.ned[1] - corner[1]);
                flight.leads.push((leg, lead));
                flight.reasons.push((leg, *reason));
            }
        }
        flight.track.push((aircraft.ned, leg));
        if output.state == MissionState::Complete {
            let completed = *flight.completed_at.get_or_insert(flight.track.len());
            if flight.track.len() >= completed + ticks_after_complete {
                break;
            }
        }
        aircraft.step(demand);
    }
    flight
}

/// The greatest distance the aircraft swings outside the leg it turned
/// onto at `corner`, while it flies that leg: positive to the left of a
/// clockwise square's next side.
fn overshoot_m(flight: &Flight, corner: usize) -> f64 {
    let corners = corners();
    let from = corners[corner];
    let to = corners[corner + 1];
    let (dn, de) = (to[0] - from[0], to[1] - from[1]);
    let length = dn.hypot(de);
    let completed = flight.completed_at.unwrap_or(flight.track.len());
    flight
        .track
        .get(..completed)
        .unwrap_or_default()
        .iter()
        .filter(|(_, leg)| *leg == corner + 1)
        .map(|(ned, _)| -((ned[1] - from[1]) * dn - (ned[0] - from[0]) * de) / length)
        .fold(f64::NEG_INFINITY, f64::max)
}

#[test]
fn each_turn_rolls_out_on_the_next_leg() {
    let (mut engine, _) = MissionEngine::from_waypoints(&plan(), config()).expect("builds");
    let flight = fly(&mut engine, 0);
    let overshoots: Vec<f64> = (1..4).map(|corner| overshoot_m(&flight, corner)).collect();
    let summary = format!(
        "turn leads {:?}, overshoots {overshoots:.1?} m",
        flight.leads
    );
    assert!(
        flight.completed_at.is_some(),
        "the plan completes: {summary}"
    );
    for overshoot in &overshoots {
        assert!(
            *overshoot <= 20.0,
            "a turn swung past the next leg: {summary}"
        );
    }
}

#[test]
fn the_true_airspeed_of_an_indicated_one_grows_with_height() {
    assert!((true_airspeed_for(CRUISE_IAS_MPS, 0.0) - CRUISE_IAS_MPS).abs() < 1e-9);
    // ISA density at 2000 m is 82.2 % of sea level's: 55 / √0.822.
    let at_2000 = true_airspeed_for(CRUISE_IAS_MPS, ALTITUDE_M);
    assert!((at_2000 - 60.66).abs() < 0.05, "{at_2000} m/s");
    // Above the tropopause the density keeps falling.
    assert!(true_airspeed_for(50.0, 12_000.0) > true_airspeed_for(50.0, 11_000.0));
    assert!(true_airspeed_for(f64::NAN, 1_000.0).is_nan());
}

/// A plan whose first waypoint lies 2 km behind the aircraft, on the
/// first leg's line: north along the first leg, then east.
fn behind_plan_points() -> [[f64; 2]; 3] {
    [[0.0, 0.0], [6_000.0, 0.0], [6_000.0, 4_000.0]]
}

/// The closest the aircraft came to the first waypoint.
fn closest_to_first_m(flight: &Flight) -> f64 {
    flight
        .track
        .iter()
        .map(|(ned, _)| ned[0].hypot(ned[1]))
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn an_airborne_fixed_wing_past_the_first_waypoint_joins_the_first_leg_ahead() {
    let points = behind_plan_points();
    let (mut engine, _) =
        MissionEngine::from_waypoints(&plan_through(&points), config()).expect("builds");
    let flight = fly_from(&mut engine, [2_000.0, 0.0], &points, 2_400, 0);
    assert!(
        matches!(
            flight.reasons.first(),
            Some((1, SequenceReason::Passed { cross_track_m })) if cross_track_m.abs() < 1.0
        ),
        "the first waypoint is passed at the start: {:?}",
        flight.reasons
    );
    let closest = closest_to_first_m(&flight);
    assert!(
        closest > 1_900.0,
        "the aircraft turned back to {closest:.0} m of the first waypoint"
    );
    // It keeps flying the first leg's way, north, to the second waypoint.
    assert!(
        flight
            .track
            .windows(2)
            .take_while(|pair| pair[1].1 == 1)
            .all(|pair| pair[1].0[0] > pair[0].0[0]),
        "the aircraft flew south on the first leg"
    );
}

#[test]
fn a_first_waypoint_marked_fly_over_is_turned_back_for() {
    let points = behind_plan_points();
    let mut plan = plan_through(&points);
    if let Some(first) = plan.waypoints.first_mut() {
        first.turn = WaypointTurn::FlyOver;
    }
    let (mut engine, record) = MissionEngine::from_waypoints(&plan, config()).expect("builds");
    let flight = fly_from(&mut engine, [2_000.0, 0.0], &points, 6_000, 0);
    assert!(
        !matches!(
            flight.reasons.first(),
            Some((1, SequenceReason::Passed { .. }))
        ),
        "a fly-over first waypoint is not passed at the start: {:?}",
        flight.reasons
    );
    let closest = closest_to_first_m(&flight);
    assert!(
        closest < record.capture_radius_m,
        "the aircraft came to {closest:.0} m of the fly-over waypoint, not over it"
    );
}

#[test]
fn a_fixed_wing_short_of_the_first_waypoint_flies_to_it() {
    let points = behind_plan_points();
    let (mut engine, record) =
        MissionEngine::from_waypoints(&plan_through(&points), config()).expect("builds");
    let flight = fly_from(&mut engine, [-2_000.0, 0.0], &points, 2_400, 0);
    assert!(
        matches!(flight.reasons.first(), Some((1, reason)) if !matches!(reason, SequenceReason::Passed { .. })),
        "the first waypoint ahead is flown to: {:?}",
        flight.reasons
    );
    assert!(closest_to_first_m(&flight) < record.capture_radius_m);
}
