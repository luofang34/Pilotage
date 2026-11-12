#![cfg(test)]

//! A fixed wing on the onboard navigator that loses its solution on a
//! leg: it holds its heading for the configured bound, then flies a
//! racetrack by heading and time near where the bound ran out, and
//! rejoins the leg when the solution comes back. It never flies a
//! straight line without end.

use core::f64::consts::TAU;

use navigate_contract::{
    AttitudeQuaternion, ClockDomainId, Ellipsoid, FaultDetection, GeodeticPosition,
    IntegrityAssessment, MonotonicNanos, NavigationSolution, NedVelocity, Redundancy, SensorClass,
    SolutionQuality, SolutionStamp, SourceComposition, SourceEpoch, SymmetricCov3,
    WrappingSequence,
};
use navigate_geodesy::NedOffset;
use navigate_geodesy::wgs84::tangent_plane;
use pilotage_mission::{
    ExecutionTarget, LateralMode, MissionConfig, MissionEngine, MissionEvent, OwnshipSource,
    PlannedWaypoint, WaypointPlan,
};
use pilotage_protocol::{ControlAction, VehicleClass};

#[path = "fixed_wing_denied/aircraft.rs"]
mod aircraft;
use aircraft::Aircraft;

const CLOCK: ClockDomainId = ClockDomainId::new(7);
const STEP_NANOS: u64 = 50_000_000;
const STEP_S: f64 = 0.05;
const ALTITUDE_M: f64 = 2_000.0;
const CRUISE_IAS_MPS: f64 = 55.0;
const GRAVITY_MPS2: f64 = 9.806_65;
const ROUTE_BANK_RAD: f64 = 0.44;
const COURSE_TIME_CONSTANT_S: f64 = 3.0;
const ROLL_TIME_CONSTANT_S: f64 = 0.5;
const MAX_ROLL_RATE_RPS: f64 = 0.5;

/// The bound of the heading hold, seconds: the default.
const BOUND_S: f64 = 60.0;
/// How far from where the bound ran out the racetrack may reach, metres.
/// A one minute leg at this airspeed is 3.6 km and the racetrack is two
/// turn radii wide, so its far corner is about 5.3 km away.
const RACETRACK_REACH_M: f64 = 6_000.0;

fn anchor() -> GeodeticPosition {
    GeodeticPosition::new(40.0_f64.to_radians(), -75.0_f64.to_radians(), ALTITUDE_M)
}

/// A leg 60 km north, then east: long enough to lose the solution on.
fn engine() -> MissionEngine {
    let mut config = MissionConfig::new(String::new(), anchor(), CLOCK);
    config.ownship_source = OwnshipSource::onboard_navigation();
    config.execution_target = ExecutionTarget::Simulator;
    config.vehicle_class = VehicleClass::FixedWing;
    config.cruise_height_m = 0.0;
    config.cruise_mps = CRUISE_IAS_MPS;
    config.limits.max_horizontal_mps = 65.0;
    config.limits.max_vertical_mps = 5.0;
    config.route_bank_deg = 22.0;
    let plane = tangent_plane(anchor()).expect("anchor is plausible");
    let plan = WaypointPlan::new(
        [[0.0, 0.0], [60_000.0, 0.0], [60_000.0, 6_000.0]]
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
    );
    MissionEngine::from_waypoints(&plan, config)
        .expect("builds")
        .0
}

fn solve(aircraft: &Aircraft, now: MonotonicNanos, sequence: u32) -> NavigationSolution {
    let plane = tangent_plane(anchor()).expect("anchor is plausible");
    let position = plane.from_ned(&NedOffset::new(
        aircraft.ned[0],
        aircraft.ned[1],
        aircraft.ned[2],
    ));
    let velocity = aircraft.ground_velocity();
    let composition = SourceComposition::of(SensorClass::VisualLandmark);
    let half = aircraft.heading_rad / 2.0;
    NavigationSolution::new(
        SolutionStamp::new(
            SourceEpoch::new(0),
            WrappingSequence::new(sequence),
            now,
            CLOCK,
        ),
        position,
        NedVelocity::new(velocity[0], velocity[1], velocity[2]),
        SymmetricCov3::from_diagonal(25.0, 25.0, 25.0),
        SymmetricCov3::from_diagonal(0.25, 0.25, 0.25),
        IntegrityAssessment::new(
            SolutionQuality::Good,
            composition,
            Redundancy::None,
            5.0,
            5.0,
            FaultDetection::Unavailable,
        ),
        composition,
        Ellipsoid::WGS84,
    )
    .with_attitude(AttitudeQuaternion::new(half.cos(), 0.0, 0.0, half.sin()))
}

/// One flight: the aircraft, the engine, and what each tick left.
struct Flight {
    engine: MissionEngine,
    aircraft: Aircraft,
    now: MonotonicNanos,
    tick: u32,
    events: Vec<MissionEvent>,
}

impl Flight {
    fn new() -> Self {
        Self {
            engine: engine(),
            aircraft: Aircraft {
                ned: [-1_500.0, 0.0, 0.0],
                heading_rad: 0.0,
                bank_rad: 0.0,
                indicated_mps: CRUISE_IAS_MPS,
                wind: [0.0, 0.0],
                heading_bias_rad: 0.0,
            },
            now: MonotonicNanos::from_nanos(1_000_000_000),
            tick: 0,
            events: Vec::new(),
        }
    }

    /// One tick, with the navigator's solution offered or not. Returns
    /// the lateral mode flown; every tick in flight must carry a command.
    fn step(&mut self, solved: bool) -> LateralMode {
        self.now = MonotonicNanos::from_nanos(self.now.as_nanos() + STEP_NANOS);
        self.tick += 1;
        if solved {
            let solution = solve(&self.aircraft, self.now, self.tick);
            self.engine.on_onboard_solution(&solution, self.now);
        }
        let output = self.engine.tick(self.now);
        if let Some(action) = output.action {
            assert_eq!(action.action, ControlAction::Arm);
            self.engine.on_action_result(action.action_id, true);
        }
        self.events.extend(output.events);
        let mode = self.engine.navigation_display().lateral.mode;
        if let Some(intent) = output.intent.as_ref() {
            self.aircraft.fly(intent);
        } else {
            assert_eq!(mode, LateralMode::Off, "a tick in flight with no command");
        }
        self.aircraft.step();
        mode
    }

    fn distance_from(&self, point: [f64; 2]) -> f64 {
        (self.aircraft.ned[0] - point[0]).hypot(self.aircraft.ned[1] - point[1])
    }
}

/// Flies with the navigator silent until the heading hold ends. Returns
/// the heading held and for how long it was, seconds.
fn hold_the_heading(flight: &mut Flight) -> (f64, f64) {
    let mut mode = LateralMode::Track;
    while mode != LateralMode::HeadingHold {
        mode = flight.step(false);
    }
    let held_heading = flight.aircraft.heading_rad;
    let mut held_s = 0.0;
    while mode == LateralMode::HeadingHold {
        assert!(
            (flight.aircraft.heading_rad - held_heading).abs() < 0.02,
            "the heading is held"
        );
        mode = flight.step(false);
        held_s += STEP_S;
    }
    assert_eq!(mode, LateralMode::TimedHold);
    (held_heading, held_s)
}

/// Flies five circuits of the racetrack with the navigator silent.
/// Returns how far the aircraft went from where it began them, metres.
fn fly_five_circuits(flight: &mut Flight) -> f64 {
    let expiry = [flight.aircraft.ned[0], flight.aircraft.ned[1]];
    let (mut turned, mut heading, mut farthest_m) = (0.0, flight.aircraft.heading_rad, 0.0_f64);
    let mut lost_s = 0.0;
    while turned < 5.0 * TAU && lost_s < 2_400.0 {
        assert_eq!(flight.step(false), LateralMode::TimedHold);
        let change = flight.aircraft.heading_rad - heading;
        turned += change.sin().atan2(change.cos());
        heading = flight.aircraft.heading_rad;
        farthest_m = farthest_m.max(flight.distance_from(expiry));
        lost_s += STEP_S;
    }
    assert!(
        turned >= 5.0 * TAU,
        "turned {:.1} times in {lost_s:.0} s",
        turned / TAU
    );
    farthest_m
}

/// The racetracks announced: each inbound course and leg time.
fn racetracks_started(flight: &Flight) -> Vec<(f64, f64)> {
    flight
        .events
        .iter()
        .filter_map(|event| match event {
            MissionEvent::TimedHoldStarted {
                inbound_course_rad,
                leg_time_s,
            } => Some((*inbound_course_rad, *leg_time_s)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_lost_solution_on_a_leg_is_a_bounded_heading_hold_then_a_racetrack() {
    let mut flight = Flight::new();
    let mut mode = LateralMode::Off;
    for _ in 0..1_200 {
        mode = flight.step(true);
    }
    assert_eq!(mode, LateralMode::Track, "on the leg, tracking north");

    // The navigator stops solving. Once its last solution is too old the
    // heading last commanded is held, for the bound and no longer.
    let (held_heading, held_s) = hold_the_heading(&mut flight);
    assert!(
        (held_s - BOUND_S).abs() <= 2.0 * STEP_S,
        "the heading was held for {held_s:.2} s"
    );
    let started = racetracks_started(&flight);
    assert_eq!(started.len(), 1, "the racetrack is announced once");
    let (inbound, leg_time_s) = started[0];
    let off = inbound - held_heading;
    assert!(
        off.sin().atan2(off.cos()).abs() < 0.02,
        "inbound on the heading held"
    );
    assert!((leg_time_s - 60.0).abs() < 1e-9, "the standard leg time");

    // Five circuits by heading and time, all near where the bound ran out.
    let farthest_m = fly_five_circuits(&mut flight);
    assert!(
        farthest_m <= RACETRACK_REACH_M,
        "{farthest_m:.0} m from where the bound ran out"
    );
    assert!(
        (flight.aircraft.ned[2]).abs() < 20.0,
        "the altitude is held through it"
    );
    // Five circuits outlast the wind estimate's age: the racetrack keeps
    // the estimate, and says once that it rests on an old one.
    let stale = flight
        .events
        .iter()
        .filter(
            |event| matches!(event, MissionEvent::TimedHoldWindStale { age_s } if *age_s > 600.0),
        )
        .count();
    assert_eq!(stale, 1, "the stale wind is announced once");
    assert!(flight.engine.navigation_display().lateral.wind_stale);

    the_leg_is_flown_again(&mut flight);
}

/// The solution comes back: the leg is flown again, and said so.
fn the_leg_is_flown_again(flight: &mut Flight) {
    let mut mode = flight.step(true);
    for _ in 0..20 {
        mode = flight.step(true);
    }
    assert!(
        matches!(mode, LateralMode::Track | LateralMode::DirectTo),
        "{mode:?}"
    );
    assert!(
        flight.events.iter().any(|event| matches!(
            event,
            MissionEvent::LateralResumed {
                mode: LateralMode::Track | LateralMode::DirectTo
            }
        )),
        "the recovery names the mode resumed"
    );
    assert!(
        !flight.engine.navigation_display().lateral.wind_stale,
        "a racetrack no longer flown rests on no wind"
    );
    let north_at_recovery = flight.aircraft.ned[0];
    for _ in 0..6_000 {
        mode = flight.step(true);
    }
    assert_eq!(mode, LateralMode::Track);
    assert!(
        flight.aircraft.ned[1].abs() < 50.0,
        "{:.0} m off the leg five minutes after the solution came back",
        flight.aircraft.ned[1]
    );
    assert!(
        flight.aircraft.ned[0] > north_at_recovery + 10_000.0,
        "and northbound along it"
    );
}

#[test]
fn wind_estimate_is_retained_across_ten_minutes_of_timed_hold_and_only_annunciated_stale() {
    let mut flight = Flight::new();
    // Strong steady wind (8 m/s south, 10 m/s east): without wind retention,
    // 700+ s of timed hold would drift > 8.9 km downwind!
    flight.aircraft.wind = [-8.0, 10.0];
    for _ in 0..1_200 {
        let _ = flight.step(true);
    }
    let (_, held_s) = hold_the_heading(&mut flight);
    assert!((held_s - BOUND_S).abs() <= 2.0 * STEP_S);

    let farthest_m = fly_five_circuits(&mut flight);
    assert!(
        farthest_m <= RACETRACK_REACH_M + 1_500.0,
        "retaining wind estimate kept orbit bounded ({farthest_m:.0} m), never drifting 9 km downwind"
    );
    let stale_events = flight
        .events
        .iter()
        .filter(|e| matches!(e, MissionEvent::TimedHoldWindStale { age_s } if *age_s > 600.0))
        .count();
    assert_eq!(
        stale_events, 1,
        "wind staleness annunciated once after 10 min"
    );
    assert!(flight.engine.navigation_display().lateral.wind_stale);

    // Fly two more circuits well past 10 min: wind correction remains active!
    let post_stale_anchor = [flight.aircraft.ned[0], flight.aircraft.ned[1]];
    for _ in 0..4_800 {
        assert_eq!(flight.step(false), LateralMode::TimedHold);
    }
    let drift_after_stale_m = flight.distance_from(post_stale_anchor);
    assert!(
        drift_after_stale_m <= RACETRACK_REACH_M + 1_500.0,
        "wind estimate was NOT discarded after 10 min staleness annunciation ({drift_after_stale_m:.0} m)"
    );
}

fn build_velocity_steered_engine() -> MissionEngine {
    let mut config = MissionConfig::new(String::new(), anchor(), CLOCK);
    config.ownship_source = OwnshipSource::vehicle_estimate();
    config.execution_target = ExecutionTarget::Simulator;
    config.vehicle_class = VehicleClass::FixedWing;
    config.cruise_height_m = 0.0;
    config.cruise_mps = CRUISE_IAS_MPS;
    config.limits.max_horizontal_mps = 65.0;
    config.route_bank_deg = 22.0;

    let plane = tangent_plane(anchor()).expect("anchor");
    let plan = WaypointPlan::new(
        [[0.0, 0.0], [60_000.0, 0.0]]
            .iter()
            .map(|[n, e]| {
                let p = plane.from_ned(&NedOffset::new(*n, *e, 0.0));
                PlannedWaypoint::new(
                    p.latitude_rad.to_degrees(),
                    p.longitude_rad.to_degrees(),
                    ALTITUDE_M,
                )
            })
            .collect(),
    );
    MissionEngine::from_waypoints(&plan, config)
        .expect("builds")
        .0
}

fn intent_course_rad(intent: pilotage_protocol::ControlIntent, course_rad: f64) -> f64 {
    match intent {
        pilotage_protocol::ControlIntent::HeadingAltitude(d) => f64::from(d.heading_rad),
        pilotage_protocol::ControlIntent::Velocity(v) => {
            let (sin, cos) = course_rad.sin_cos();
            let vn = f64::from(v.vx) * cos - f64::from(v.vy) * sin;
            let ve = f64::from(v.vx) * sin + f64::from(v.vy) * cos;
            ve.atan2(vn)
        }
        other => panic!("unexpected intent: {other:?}"),
    }
}

fn track_initial_leg(engine: &mut MissionEngine, ned: &mut [f64; 3], now_ns: &mut u64) {
    let vel = [CRUISE_IAS_MPS, 0.0, 0.0];
    for seq in 1..=200_u32 {
        *now_ns = now_ns.wrapping_add(STEP_NANOS);
        let now = MonotonicNanos::from_nanos(*now_ns);
        let sample = pilotage_mission::OwnshipSample {
            ned: *ned,
            ned_velocity: vel,
            yaw_rad: Some(0.0),
            role: pilotage_mission::TruthRole::OperationalEstimate,
            acquired_at: now,
            sequence: seq,
        };
        engine.on_ownship(&sample, now);
        let out = engine.tick(now);
        if let Some(act) = out.action
            && act.action == ControlAction::Arm
        {
            engine.on_action_result(act.action_id, true);
        }
        ned[0] += vel[0] * STEP_S;
    }
}

#[test]
fn velocity_steered_fixed_wing_holds_heading_sixty_seconds_then_orbits_never_coasting_straight() {
    let mut engine = build_velocity_steered_engine();
    let mut ned = [100.0_f64, 0.0, 0.0];
    let mut course_rad = 0.0_f64;
    let mut now_ns = 1_000_000_000_u64;

    track_initial_leg(&mut engine, &mut ned, &mut now_ns);
    assert_eq!(engine.navigation_display().lateral.mode, LateralMode::Track);

    // 2. Lose position completely: verify 60 s HeadingHold -> TimedHold orbit!
    let mut held_s = 0.0_f64;
    let mut mode = LateralMode::Track;
    while mode != LateralMode::TimedHold {
        now_ns = now_ns.wrapping_add(STEP_NANOS);
        let out = engine.tick(MonotonicNanos::from_nanos(now_ns));
        assert!(out.intent.is_some(), "fixed wing always receives a command");
        mode = engine.navigation_display().lateral.mode;
        if mode == LateralMode::HeadingHold {
            held_s += STEP_S;
        }
        ned[0] += CRUISE_IAS_MPS * course_rad.cos() * STEP_S;
        ned[1] += CRUISE_IAS_MPS * course_rad.sin() * STEP_S;
    }
    assert!(
        (held_s - BOUND_S).abs() <= 2.0 * STEP_S,
        "held heading for {held_s:.2} s"
    );

    // 3. Follow commands in TimedHold and confirm continuous closed orbit!
    let orbit_origin = [ned[0], ned[1]];
    let mut turned = 0.0_f64;
    let mut farthest_m = 0.0_f64;
    for _ in 0..10_000 {
        now_ns = now_ns.wrapping_add(STEP_NANOS);
        let out = engine.tick(MonotonicNanos::from_nanos(now_ns));
        assert_eq!(
            engine.navigation_display().lateral.mode,
            LateralMode::TimedHold
        );
        let next_course = intent_course_rad(out.intent.expect("command in TimedHold"), course_rad);
        let d = (next_course - course_rad)
            .sin()
            .atan2((next_course - course_rad).cos());
        turned += d;
        course_rad = next_course;
        ned[0] += CRUISE_IAS_MPS * course_rad.cos() * STEP_S;
        ned[1] += CRUISE_IAS_MPS * course_rad.sin() * STEP_S;
        farthest_m = farthest_m.max((ned[0] - orbit_origin[0]).hypot(ned[1] - orbit_origin[1]));
    }
    assert!(
        turned.abs() > 2.0 * TAU,
        "completed multiple orbits ({:.2} rad)",
        turned
    );
    assert!(
        farthest_m < RACETRACK_REACH_M,
        "stayed bounded ({farthest_m:.0} m)"
    );
}
