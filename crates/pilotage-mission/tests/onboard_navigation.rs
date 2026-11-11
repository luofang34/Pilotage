#![cfg(test)]

//! The onboard-navigation ownship source: the engine flies the solution
//! of the vehicle's navigator, without fusion, and stops when that
//! solution is old, unfit, or absent.
//!
//! Stand-ins. The vehicle is a point that follows each body-frame
//! velocity command exactly. The navigator is the vehicle's position plus
//! a fixed offset, with the vehicle's heading as its attitude. Only the
//! navigator's solution reaches the engine.

use navigate_contract::{
    AttitudeQuaternion, ClockDomainId, Ellipsoid, FaultDetection, GeodeticPosition,
    IntegrityAssessment, MonotonicNanos, NavigationSolution, NedVelocity, Redundancy, SensorClass,
    SolutionQuality, SolutionStamp, SourceComposition, SourceEpoch, SymmetricCov3,
    WrappingSequence,
};
use navigate_geodesy::{BodyEllipsoid, LocalTangentPlane, NedOffset};
use pilotage_mission::fixture::{self, GeoPointDegrees};
use pilotage_mission::{
    MissionConfig, MissionEngine, MissionEvent, MissionOutput, MissionState, OwnshipSample,
    OwnshipSource, StartWithheldReason, TruthRole, decode_snapshot,
};
use pilotage_protocol::{ControlAction, ControlIntent, ReferenceFrame};

const ANCHOR: GeoPointDegrees = GeoPointDegrees {
    lat_deg: 47.0,
    lon_deg: 8.0,
    alt_m: 500.0,
};
const CLOCK: ClockDomainId = ClockDomainId::new(7);
const NAVIGATOR_CLOCK: ClockDomainId = ClockDomainId::new(42);
const STEP_NANOS: u64 = 50_000_000;
const STEP_S: f64 = 0.05;
const START_NANOS: u64 = 1_000_000_000;

fn anchor() -> GeodeticPosition {
    GeodeticPosition::new(
        ANCHOR.lat_deg.to_radians(),
        ANCHOR.lon_deg.to_radians(),
        ANCHOR.alt_m,
    )
}

/// The point vehicle: NED position from the anchor and heading.
struct Vehicle {
    ned: [f64; 3],
    ned_velocity: [f64; 3],
    yaw: f64,
}

impl Vehicle {
    fn new() -> Self {
        Self {
            ned: [0.0; 3],
            ned_velocity: [0.0; 3],
            yaw: 0.0,
        }
    }

    fn fly(&mut self, intent: &ControlIntent) {
        let ControlIntent::Velocity(v) = intent else {
            panic!("the mission commands velocity under this source, got {intent:?}");
        };
        assert_eq!(v.frame, ReferenceFrame::BodyFrd);
        let (vx, vy, vz) = (f64::from(v.vx), f64::from(v.vy), f64::from(v.vz));
        let (sin, cos) = self.yaw.sin_cos();
        self.ned_velocity = [vx * cos - vy * sin, vx * sin + vy * cos, vz];
        for (position, velocity) in self.ned.iter_mut().zip(self.ned_velocity) {
            *position += velocity * STEP_S;
        }
        self.yaw += f64::from(v.yaw_rate) * STEP_S;
    }
}

/// The navigator stand-in: what it adds to the vehicle's position, how it
/// grades its solution, and whether it knows the attitude.
struct Navigator {
    plane: LocalTangentPlane,
    offset_ned_m: [f64; 3],
    quality: SolutionQuality,
    horizontal_1sigma_m: f64,
    with_attitude: bool,
    sequence: u32,
}

impl Navigator {
    fn new() -> Self {
        Self {
            plane: LocalTangentPlane::new(anchor(), BodyEllipsoid::WGS84).expect("anchor plane"),
            offset_ned_m: [0.0; 3],
            quality: SolutionQuality::Good,
            horizontal_1sigma_m: 2.0,
            with_attitude: true,
            sequence: 0,
        }
    }

    /// A new solution for the vehicle, stamped `solved_at` on `clock`.
    fn solve(
        &mut self,
        vehicle: &Vehicle,
        solved_at: MonotonicNanos,
        clock: ClockDomainId,
    ) -> NavigationSolution {
        self.sequence = self.sequence.wrapping_add(1);
        let position = self.plane.from_ned(&NedOffset::new(
            vehicle.ned[0] + self.offset_ned_m[0],
            vehicle.ned[1] + self.offset_ned_m[1],
            vehicle.ned[2] + self.offset_ned_m[2],
        ));
        let composition = SourceComposition::of(SensorClass::Gnss);
        let variance = self.horizontal_1sigma_m * self.horizontal_1sigma_m;
        let solution = NavigationSolution::new(
            SolutionStamp::new(
                SourceEpoch::new(0),
                WrappingSequence::new(self.sequence),
                solved_at,
                clock,
            ),
            position,
            NedVelocity::new(
                vehicle.ned_velocity[0],
                vehicle.ned_velocity[1],
                vehicle.ned_velocity[2],
            ),
            SymmetricCov3::from_diagonal(variance, variance, variance),
            SymmetricCov3::from_diagonal(0.25, 0.25, 0.25),
            IntegrityAssessment::new(
                self.quality,
                composition,
                Redundancy::None,
                self.horizontal_1sigma_m,
                self.horizontal_1sigma_m,
                FaultDetection::Unavailable,
            ),
            composition,
            Ellipsoid::WGS84,
        );
        if self.with_attitude {
            let half = vehicle.yaw / 2.0;
            solution.with_attitude(AttitudeQuaternion::new(half.cos(), 0.0, 0.0, half.sin()))
        } else {
            solution
        }
    }
}

fn build_engine() -> MissionEngine {
    let blob = fixture::demo_blob(ANCHOR).expect("demo blob encodes");
    let (snapshot, provenance) = decode_snapshot(&blob, true).expect("demo blob decodes");
    let mut config = MissionConfig::new(fixture::DEMO_ROUTE.to_owned(), anchor(), CLOCK);
    config.ownship_source = OwnshipSource::onboard_navigation();
    MissionEngine::new(&snapshot, provenance, config)
        .expect("mission builds")
        .0
}

/// One tick of the engine at `now`; the vehicle flies what it commands.
fn tick(engine: &mut MissionEngine, vehicle: &mut Vehicle, now: MonotonicNanos) -> MissionOutput {
    let out = engine.tick(now);
    if let Some(action) = out.action {
        assert!(matches!(action.action, ControlAction::Arm));
        engine.on_action_result(action.action_id, true);
    }
    if let Some(intent) = out.intent.as_ref() {
        vehicle.fly(intent);
    }
    out
}

/// Flies the demo route on the navigator's solutions, stamped on the
/// mission clock, until `done` holds or the tick budget ends. Returns
/// the last output and the time of the last tick.
fn fly_until(
    engine: &mut MissionEngine,
    vehicle: &mut Vehicle,
    navigator: &mut Navigator,
    done: impl Fn(&MissionOutput) -> bool,
) -> (MissionOutput, MonotonicNanos) {
    let mut now = MonotonicNanos::from_nanos(START_NANOS);
    for _ in 0..40_000 {
        now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
        engine.on_onboard_solution(&navigator.solve(vehicle, now, CLOCK), now);
        let out = tick(engine, vehicle, now);
        if done(&out) {
            return (out, now);
        }
    }
    panic!("the condition did not hold inside the tick budget");
}

fn complete(out: &MissionOutput) -> bool {
    out.state == MissionState::Complete
}

fn enroute(out: &MissionOutput) -> bool {
    out.state == MissionState::Enroute && out.intent.is_some()
}

#[test]
fn the_vehicle_flies_where_the_navigator_says_it_is() {
    let mut reference = Vehicle::new();
    fly_until(
        &mut build_engine(),
        &mut reference,
        &mut Navigator::new(),
        complete,
    );

    // The navigator reads 30 m east of the vehicle, so the vehicle must
    // end 30 m west of where a correct navigator takes it.
    let mut shifted = Vehicle::new();
    let mut navigator = Navigator::new();
    navigator.offset_ned_m = [0.0, 30.0, 0.0];
    let mut engine = build_engine();
    fly_until(&mut engine, &mut shifted, &mut navigator, complete);

    let north = shifted.ned[0] - reference.ned[0];
    let east = shifted.ned[1] - reference.ned[1];
    assert!(
        north.abs() < 1.0 && (east + 30.0).abs() < 1.0,
        "the end point moved by ({north:.2}, {east:.2}) m, expected (0, -30)"
    );
    assert_eq!(engine.counters().onboard_refused, 0);
    assert_eq!(engine.counters().onboard_stale, 0);
    assert_eq!(engine.counters().onboard_unfit, 0);
}

#[test]
fn a_navigator_that_stops_solving_goes_stale_while_its_host_repeats_the_solution() {
    let mut engine = build_engine();
    let mut vehicle = Vehicle::new();
    let mut navigator = Navigator::new();
    let (_, mut now) = fly_until(&mut engine, &mut vehicle, &mut navigator, enroute);

    let last = navigator.solve(&vehicle, now, CLOCK);
    engine.on_onboard_solution(&last, now);
    let mut silent_ticks = 0_u32;
    for _ in 0..40 {
        now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
        engine.on_onboard_solution(&last, now);
        let out = tick(&mut engine, &mut vehicle, now);
        if out.intent.is_none() {
            silent_ticks += 1;
            assert!(engine.nav_guidance().is_none());
        }
    }

    // One second of age is 20 ticks: the first 20 ticks fly the held
    // solution and the next 20 command nothing.
    assert_eq!(silent_ticks, 20);
    assert_eq!(engine.counters().onboard_repeated, 40);
    assert_eq!(engine.counters().onboard_stale, 20);
}

#[test]
fn an_unusable_or_uncertain_solution_is_not_flown() {
    let mut engine = build_engine();
    let vehicle = Vehicle::new();
    let mut navigator = Navigator::new();
    let mut now = MonotonicNanos::from_nanos(START_NANOS);
    let mut offer = |navigator: &mut Navigator, engine: &mut MissionEngine| {
        now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
        engine.on_onboard_solution(&navigator.solve(&vehicle, now, CLOCK), now);
        engine.tick(now)
    };

    navigator.quality = SolutionQuality::Unusable;
    let out = offer(&mut navigator, &mut engine);
    assert_eq!(out.state, MissionState::AwaitSolution);
    assert_eq!(engine.counters().onboard_unfit, 1);

    navigator.quality = SolutionQuality::Degraded;
    navigator.horizontal_1sigma_m = 50.5;
    let out = offer(&mut navigator, &mut engine);
    assert_eq!(out.state, MissionState::AwaitSolution);
    assert_eq!(engine.counters().onboard_unfit, 2);

    navigator.horizontal_1sigma_m = 50.0;
    let out = offer(&mut navigator, &mut engine);
    assert_ne!(out.state, MissionState::AwaitSolution);
    assert_eq!(engine.counters().onboard_unfit, 2);
}

#[test]
fn a_solution_on_another_clock_ages_from_its_offer() {
    let mut engine = build_engine();
    let mut vehicle = Vehicle::new();
    let mut navigator = Navigator::new();
    let mut now = MonotonicNanos::from_nanos(START_NANOS);
    // The navigator's clock reads far ahead of the mission clock. The two
    // readings are not comparable, so the stamp is not read as an age.
    let navigator_time = MonotonicNanos::from_nanos(900_000_000_000);
    let mut reached_enroute = false;
    for _ in 0..4_000 {
        now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
        let solution = navigator.solve(&vehicle, navigator_time, NAVIGATOR_CLOCK);
        engine.on_onboard_solution(&solution, now);
        if enroute(&tick(&mut engine, &mut vehicle, now)) {
            reached_enroute = true;
            break;
        }
    }
    assert!(
        reached_enroute,
        "guidance must accept the restamped solution"
    );
    assert_eq!(engine.counters().onboard_refused, 0);
    assert_eq!(engine.counters().guidance_refused, 0);
}

#[test]
fn a_solution_the_engine_cannot_place_is_a_counted_refusal() {
    let mut engine = build_engine();
    let vehicle = Vehicle::new();
    let mut navigator = Navigator::new();
    let now = MonotonicNanos::from_nanos(START_NANOS);

    let later = MonotonicNanos::from_nanos(START_NANOS + 1);
    engine.on_onboard_solution(&navigator.solve(&vehicle, later, CLOCK), now);
    let moon = navigator
        .solve(&vehicle, now, CLOCK)
        .with_ellipsoid(Ellipsoid::sphere(Ellipsoid::WGS84.frame, 1_737_400.0));
    engine.on_onboard_solution(&moon, now);
    let mut nowhere = navigator.solve(&vehicle, now, CLOCK);
    nowhere.position.latitude_rad = f64::NAN;
    engine.on_onboard_solution(&nowhere, now);
    let mut unmoving = navigator.solve(&vehicle, now, CLOCK);
    unmoving.velocity.east_mps = f64::INFINITY;
    engine.on_onboard_solution(&unmoving, now);

    assert_eq!(engine.counters().onboard_refused, 4);
    assert_eq!(engine.tick(now).state, MissionState::AwaitSolution);
    assert_eq!(engine.counters().onboard_stale, 0);
}

#[test]
fn ownship_samples_are_refused_under_the_onboard_source() {
    let mut engine = build_engine();
    let now = MonotonicNanos::from_nanos(START_NANOS);
    for role in [
        TruthRole::SimulationTruth,
        TruthRole::OperationalEstimate,
        TruthRole::FcState,
    ] {
        let sample = OwnshipSample {
            ned: [0.0; 3],
            ned_velocity: [0.0; 3],
            yaw_rad: Some(0.0),
            role,
            acquired_at: now,
            sequence: 1,
        };
        engine.on_ownship(&sample, now);
    }
    assert_eq!(engine.counters().rejected_role, 3);
    assert_eq!(engine.tick(now).state, MissionState::AwaitSolution);
}

#[test]
fn a_solution_without_an_attitude_gives_no_body_frame_command() {
    let mut engine = build_engine();
    let mut vehicle = Vehicle::new();
    let mut navigator = Navigator::new();
    navigator.with_attitude = false;
    let (out, _) = fly_until(&mut engine, &mut vehicle, &mut navigator, |out| {
        out.state == MissionState::Enroute
    });
    assert!(out.intent.is_none());
    assert!(engine.counters().missing_yaw > 0);
}

#[test]
fn a_start_without_a_solution_to_fly_says_why_once_for_each_reason() {
    let mut engine = build_engine();
    let vehicle = Vehicle::new();
    let mut navigator = Navigator::new();
    let mut now = MonotonicNanos::from_nanos(START_NANOS);
    let mut withheld = Vec::new();
    let mut step = |engine: &mut MissionEngine, solution: Option<&mut Navigator>| {
        now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
        if let Some(navigator) = solution {
            engine.on_onboard_solution(&navigator.solve(&vehicle, now, CLOCK), now);
        }
        engine.tick(now)
    };
    let mut record = |out: &MissionOutput| {
        for event in &out.events {
            if let MissionEvent::StartWithheld { reason } = event {
                withheld.push(*reason);
            }
        }
    };

    for _ in 0..10 {
        record(&step(&mut engine, None));
    }
    navigator.horizontal_1sigma_m = 80.0;
    for sigma in [80.0, 95.0, 120.0] {
        navigator.horizontal_1sigma_m = sigma;
        record(&step(&mut engine, Some(&mut navigator)));
    }
    navigator.quality = SolutionQuality::Unusable;
    record(&step(&mut engine, Some(&mut navigator)));
    navigator.quality = SolutionQuality::Good;
    navigator.horizontal_1sigma_m = 2.0;
    let out = step(&mut engine, Some(&mut navigator));
    record(&out);

    assert_eq!(
        withheld,
        [
            StartWithheldReason::NavigationNotStarted,
            StartWithheldReason::NavigationTooUncertain {
                horizontal_1sigma_m: 80.0,
                limit_m: 50.0
            },
            StartWithheldReason::NavigationUnusable,
        ],
        "each reason once, in order; a changed uncertainty is not a new reason"
    );
    assert!(out.action.is_some(), "a fit solution starts the mission");
}
