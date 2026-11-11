#![cfg(test)]

//! A fixed wing flown on the onboard navigator, as with no satellite
//! navigation: its flight controller holds heading, altitude, and airspeed
//! with its own sensors, so the engine steers it with heading and altitude
//! demands, corrects the heading for the wind the navigator shows, and
//! holds heading and altitude whenever the navigator has no solution fit to
//! fly instead of going silent.

use navigate_contract::{
    AttitudeQuaternion, ClockDomainId, Ellipsoid, FaultDetection, GeodeticPosition,
    IntegrityAssessment, MonotonicNanos, NavigationSolution, NedVelocity, Redundancy, SensorClass,
    SolutionQuality, SolutionStamp, SourceComposition, SourceEpoch, SymmetricCov3,
    WrappingSequence,
};
use navigate_geodesy::NedOffset;
use navigate_geodesy::wgs84::tangent_plane;
use pilotage_mission::{
    DeclaredHeading, DeclaredPosition, DeclaredStart, ExecutionTarget, LaunchDetection,
    MissionBuildError, MissionConfig, MissionEngine, MissionEvent, MissionState, OwnshipSource,
    PlannedWaypoint, StartWithheldReason, VehicleStatus, WaypointPlan, true_airspeed_for,
};
use pilotage_protocol::{AltitudeReference, ControlAction, ControlIntent, VehicleClass};

const CLOCK: ClockDomainId = ClockDomainId::new(7);
const STEP_NANOS: u64 = 50_000_000;
const STEP_S: f64 = 0.05;
const ALTITUDE_M: f64 = 2_000.0;
const CRUISE_IAS_MPS: f64 = 55.0;
const GRAVITY_MPS2: f64 = 9.806_65;
/// Aviate C172 `route_bank_angle`, `course_time_constant`,
/// `roll_time_constant`, and `max_roll_rate`.
const ROUTE_BANK_RAD: f64 = 0.44;
const COURSE_TIME_CONSTANT_S: f64 = 3.0;
const ROLL_TIME_CONSTANT_S: f64 = 0.5;
const MAX_ROLL_RATE_RPS: f64 = 0.5;
/// A wind from the north-west at 12 m/s, blowing toward the south-east.
const WIND_NED_MPS: [f64; 2] = [-8.485, 8.485];

fn anchor() -> GeodeticPosition {
    GeodeticPosition::new(40.0_f64.to_radians(), -75.0_f64.to_radians(), ALTITUDE_M)
}

fn config() -> MissionConfig {
    let mut config = MissionConfig::new(String::new(), anchor(), CLOCK);
    config.ownship_source = OwnshipSource::onboard_navigation();
    config.execution_target = ExecutionTarget::Simulator;
    config.vehicle_class = VehicleClass::FixedWing;
    config.cruise_height_m = 0.0;
    config.cruise_mps = CRUISE_IAS_MPS;
    config.limits.max_horizontal_mps = 65.0;
    config.limits.max_vertical_mps = 5.0;
    config.route_bank_deg = 22.0;
    config
}

/// North 6 km, then east 6 km.
fn points() -> [[f64; 2]; 3] {
    [[0.0, 0.0], [6_000.0, 0.0], [6_000.0, 6_000.0]]
}

fn plan() -> WaypointPlan {
    let plane = tangent_plane(anchor()).expect("anchor is plausible");
    WaypointPlan::new(
        points()
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

/// A fixed wing in a steady wind holding the heading, altitude, and
/// indicated airspeed it is commanded, with Aviate's heading-to-bank law.
struct Aircraft {
    ned: [f64; 3],
    heading_rad: f64,
    bank_rad: f64,
    indicated_mps: f64,
}

impl Aircraft {
    fn ground_velocity(&self) -> [f64; 3] {
        let airspeed = true_airspeed_for(self.indicated_mps, ALTITUDE_M - self.ned[2]);
        [
            airspeed * self.heading_rad.cos() + WIND_NED_MPS[0],
            airspeed * self.heading_rad.sin() + WIND_NED_MPS[1],
            0.0,
        ]
    }

    fn fly(&mut self, intent: &ControlIntent) {
        let ControlIntent::HeadingAltitude(demand) = intent else {
            panic!("a fixed wing on the onboard navigator flies heading and altitude: {intent:?}");
        };
        assert_eq!(demand.reference, AltitudeReference::Ellipsoid);
        if let Some(airspeed) = demand.airspeed_mps {
            self.indicated_mps = f64::from(airspeed);
        }
        let airspeed = true_airspeed_for(self.indicated_mps, ALTITUDE_M - self.ned[2]);
        let rate_per_tangent = GRAVITY_MPS2 / airspeed;
        let wanted = f64::from(demand.heading_rad);
        let error = (wanted - self.heading_rad)
            .sin()
            .atan2((wanted - self.heading_rad).cos());
        let wanted_bank = (error / COURSE_TIME_CONSTANT_S / rate_per_tangent)
            .atan()
            .clamp(-ROUTE_BANK_RAD, ROUTE_BANK_RAD);
        let roll_rate = ((wanted_bank - self.bank_rad) / ROLL_TIME_CONSTANT_S)
            .clamp(-MAX_ROLL_RATE_RPS, MAX_ROLL_RATE_RPS);
        self.bank_rad += roll_rate * STEP_S;
        self.heading_rad += rate_per_tangent * self.bank_rad.tan() * STEP_S;
        // The altitude loop: a climb or sink rate toward the demand.
        let height_error = f64::from(demand.altitude_m) - (ALTITUDE_M - self.ned[2]);
        self.ned[2] -= (height_error / 5.0).clamp(-5.0, 2.5) * STEP_S;
    }

    fn step(&mut self) {
        let velocity = self.ground_velocity();
        self.ned[0] += velocity[0] * STEP_S;
        self.ned[1] += velocity[1] * STEP_S;
    }
}

/// The onboard navigator's solution of the aircraft, with `sigma_m` of
/// horizontal uncertainty.
fn solve(
    aircraft: &Aircraft,
    now: MonotonicNanos,
    sequence: u32,
    sigma_m: f64,
) -> NavigationSolution {
    let plane = tangent_plane(anchor()).expect("anchor is plausible");
    let position = plane.from_ned(&NedOffset::new(
        aircraft.ned[0],
        aircraft.ned[1],
        aircraft.ned[2],
    ));
    let velocity = aircraft.ground_velocity();
    let composition = SourceComposition::of(SensorClass::VisualLandmark);
    let variance = sigma_m * sigma_m;
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
        SymmetricCov3::from_diagonal(variance, variance, variance),
        SymmetricCov3::from_diagonal(0.25, 0.25, 0.25),
        IntegrityAssessment::new(
            SolutionQuality::Good,
            composition,
            Redundancy::None,
            sigma_m,
            sigma_m,
            FaultDetection::Unavailable,
        ),
        composition,
        Ellipsoid::WGS84,
    )
    .with_attitude(AttitudeQuaternion::new(half.cos(), 0.0, 0.0, half.sin()))
}

/// What the navigator gives at a tick: a solution with this uncertainty,
/// or none.
type Navigator = dyn Fn(usize) -> Option<f64>;

struct Flight {
    /// The position, active state, and intent at each tick.
    track: Vec<([f64; 3], MissionState, Option<ControlIntent>)>,
    holds: Vec<StartWithheldReason>,
}

fn fly(engine: &mut MissionEngine, ticks: usize, navigator: &Navigator) -> Flight {
    let mut aircraft = Aircraft {
        ned: [-1_500.0, 0.0, 0.0],
        heading_rad: 0.0,
        bank_rad: 0.0,
        indicated_mps: CRUISE_IAS_MPS,
    };
    let mut flight = Flight {
        track: Vec::new(),
        holds: Vec::new(),
    };
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    for tick in 0..ticks {
        now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
        if let Some(sigma_m) = navigator(tick) {
            let sequence = u32::try_from(tick).expect("a short flight");
            engine.on_onboard_solution(&solve(&aircraft, now, sequence, sigma_m), now);
        }
        let output = engine.tick(now);
        if let Some(action) = output.action {
            assert_eq!(action.action, ControlAction::Arm);
            engine.on_action_result(action.action_id, true);
        }
        for event in &output.events {
            if let MissionEvent::NavigationHold { reason } = event {
                flight.holds.push(*reason);
            }
        }
        if let Some(intent) = output.intent.as_ref() {
            aircraft.fly(intent);
        }
        flight
            .track
            .push((aircraft.ned, output.state, output.intent));
        aircraft.step();
    }
    flight
}

#[test]
fn a_crosswind_is_flown_out_along_each_leg() {
    let (mut engine, _) = MissionEngine::from_waypoints(&plan(), config()).expect("builds");
    let flight = fly(&mut engine, 6_000, &|_| Some(5.0));
    // Mid first leg, north: the track holds the leg's east coordinate.
    let first_leg: Vec<f64> = flight
        .track
        .iter()
        .filter(|(ned, _, _)| (2_000.0..4_000.0).contains(&ned[0]) && ned[1] < 1_000.0)
        .map(|(ned, _, _)| ned[1].abs())
        .collect();
    assert!(!first_leg.is_empty(), "the first leg was flown");
    let worst = first_leg.iter().copied().fold(0.0, f64::max);
    assert!(
        worst < 30.0,
        "{worst:.1} m off the first leg in the crosswind"
    );
    // Mid second leg, east.
    let second_leg: Vec<f64> = flight
        .track
        .iter()
        .filter(|(ned, _, _)| (2_000.0..4_000.0).contains(&ned[1]) && ned[0] > 5_000.0)
        .map(|(ned, _, _)| (ned[0] - 6_000.0).abs())
        .collect();
    assert!(!second_leg.is_empty(), "the second leg was flown");
    let worst = second_leg.iter().copied().fold(0.0, f64::max);
    assert!(
        worst < 30.0,
        "{worst:.1} m off the second leg in the crosswind"
    );
    assert!(flight.holds.is_empty(), "{:?}", flight.holds);
}

#[test]
fn an_uncertain_or_absent_solution_holds_heading_and_altitude_and_says_why() {
    let (mut engine, _) = MissionEngine::from_waypoints(&plan(), config()).expect("builds");
    // Fit for 60 s, too uncertain for 20 s, fit for 20 s, then nothing
    // for 20 s, then fit again.
    let navigator = |tick: usize| match tick {
        0..1_200 => Some(5.0),
        1_200..1_600 => Some(80.0 + (tick % 7) as f64),
        1_600..2_000 => Some(5.0),
        2_000..2_400 => None,
        _ => Some(5.0),
    };
    let flight = fly(&mut engine, 3_000, &navigator);
    assert!(
        matches!(
            flight.holds.as_slice(),
            [
                StartWithheldReason::NavigationTooUncertain { limit_m, .. },
                StartWithheldReason::NavigationStale,
            ] if *limit_m == 50.0
        ),
        "each hold announced once, with its reason: {:?}",
        flight.holds
    );
    // A command every tick from the arm on: never silence.
    let first_command = flight
        .track
        .iter()
        .position(|(_, _, intent)| intent.is_some())
        .expect("a command");
    assert!(
        flight.track[first_command..]
            .iter()
            .all(|(_, _, intent)| matches!(intent, Some(ControlIntent::HeadingAltitude(_)))),
        "every command is a heading and altitude"
    );
    // The hold keeps one heading and one altitude.
    let held = |range: core::ops::Range<usize>| {
        let commands: Vec<(f32, f32)> = flight.track[range]
            .iter()
            .filter_map(|(_, _, intent)| match intent {
                Some(ControlIntent::HeadingAltitude(h)) => Some((h.heading_rad, h.altitude_m)),
                _ => None,
            })
            .collect();
        commands.windows(2).all(|pair| pair[0] == pair[1])
    };
    assert!(
        held(1_201..1_600),
        "the uncertain stretch holds one command"
    );
    assert!(held(2_021..2_400), "the absent stretch holds one command");
}

#[test]
fn a_launch_seen_by_airspeed_alone_gets_a_climb_on_its_heading_without_a_solution() {
    let mut config = config();
    config.cruise_height_m = 300.0;
    config.launch_detection = Some(LaunchDetection {
        min_height_m: 15.0,
        min_airspeed_mps: 30.0,
    });
    let (mut engine, _) = MissionEngine::from_waypoints(&plan(), config).expect("builds");
    let aircraft = Aircraft {
        ned: [-1_500.0, 0.0, 0.0],
        heading_rad: 0.7,
        bank_rad: 0.0,
        indicated_mps: CRUISE_IAS_MPS,
    };
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let mut status = VehicleStatus::ready(true, now);
    let mut outputs = Vec::new();
    for tick in 0..40_u32 {
        now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
        // The navigator's solution never gets fit to fly.
        engine.on_onboard_solution(&solve(&aircraft, now, tick, 400.0), now);
        status.acquired_at = now;
        status.airspeed_mps = Some(if tick < 20 { 12.0 } else { 45.0 });
        engine.on_vehicle_status(status);
        outputs.push(engine.tick(now));
    }
    assert!(
        outputs[..20].iter().all(|output| output.intent.is_none()),
        "a taxiing aircraft is not commanded"
    );
    let ControlIntent::HeadingAltitude(climb) = outputs[25]
        .intent
        .expect("a launched aircraft is commanded")
    else {
        panic!("a heading and altitude");
    };
    assert!(
        (f64::from(climb.heading_rad) - 0.7).abs() < 1e-6,
        "{climb:?}"
    );
    assert_eq!(f64::from(climb.altitude_m), ALTITUDE_M + 300.0);
    assert!(
        outputs
            .iter()
            .any(|output| output.events.iter().any(|event| matches!(
                event,
                MissionEvent::NavigationHold {
                    reason: StartWithheldReason::NavigationTooUncertain { .. }
                }
            )))
    );
}

#[test]
fn launch_detection_takes_either_the_height_or_the_airspeed() {
    let detection = LaunchDetection {
        min_height_m: 15.0,
        min_airspeed_mps: 30.0,
    };
    let at = MonotonicNanos::from_nanos(0);
    let mut status = VehicleStatus::ready(true, at);
    assert!(!detection.launched(&status), "no measurement, no launch");
    status.height_above_origin_m = Some(14.0);
    status.airspeed_mps = Some(29.0);
    assert!(!detection.launched(&status));
    status.height_above_origin_m = Some(15.0);
    assert!(detection.launched(&status));
    status.height_above_origin_m = None;
    status.airspeed_mps = Some(31.0);
    assert!(detection.launched(&status));
}

#[test]
fn a_declared_start_is_handed_on_as_stated_and_an_unusable_one_refused() {
    let declared = DeclaredStart {
        position: Some(DeclaredPosition {
            latitude_rad: 40.0_f64.to_radians(),
            longitude_rad: -75.0_f64.to_radians(),
            sigma_m: 50.0,
        }),
        height: None,
        heading: Some(DeclaredHeading {
            heading_rad: 1.0,
            sigma_rad: 0.2,
        }),
        speed: None,
        declared_at: MonotonicNanos::from_nanos(7),
    };
    let mut config = config();
    config.declared_start = Some(declared);
    let (engine, _) = MissionEngine::from_waypoints(&plan(), config.clone()).expect("builds");
    assert_eq!(engine.declared_start(), Some(&declared));

    let mut unusable = declared;
    unusable.heading = Some(DeclaredHeading {
        heading_rad: 1.0,
        sigma_rad: 0.0,
    });
    config.declared_start = Some(unusable);
    assert!(matches!(
        MissionEngine::from_waypoints(&plan(), config),
        Err(MissionBuildError::DeclaredStart { source }) if source.field == "heading.sigma_rad"
    ));
}
