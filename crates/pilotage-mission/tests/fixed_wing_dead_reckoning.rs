#![cfg(test)]

//! A fixed wing flown on a navigator with no position aid, as an inertial
//! and barometric one without satellites: its horizontal uncertainty grows
//! for the whole flight and its altitude sits a few metres off. The plan
//! must still be flown, by dead reckoning, with the engine saying so; and
//! the climb must capture although the navigator's altitude never comes
//! within a metre of the target.

use navigate_contract::{
    AttitudeQuaternion, ClockDomainId, Ellipsoid, FaultDetection, GeodeticPosition,
    IntegrityAssessment, MonotonicNanos, NavigationSolution, NedVelocity, Redundancy, SensorClass,
    SolutionQuality, SolutionStamp, SourceComposition, SourceEpoch, SymmetricCov3,
    WrappingSequence,
};
use navigate_geodesy::NedOffset;
use navigate_geodesy::wgs84::tangent_plane;
use pilotage_mission::{
    ExecutionTarget, FlightPhase, MissionConfig, MissionEngine, MissionEvent, MissionState,
    NavQuality, OwnshipSource, PlannedWaypoint, PositionSource, StartWithheldReason,
    SteeringIntegrity, VehicleStatus, WaypointPlan,
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
/// Aviate C172 `route_bank_angle`, `course_time_constant`,
/// `roll_time_constant`, and `max_roll_rate`.
const ROUTE_BANK_RAD: f64 = 0.44;
const COURSE_TIME_CONSTANT_S: f64 = 3.0;
const ROLL_TIME_CONSTANT_S: f64 = 0.5;
const MAX_ROLL_RATE_RPS: f64 = 0.5;

fn anchor() -> GeodeticPosition {
    GeodeticPosition::new(40.0_f64.to_radians(), -75.0_f64.to_radians(), ALTITUDE_M)
}

fn config(source: OwnshipSource, cruise_height_m: f64) -> MissionConfig {
    let mut config = MissionConfig::new(String::new(), anchor(), CLOCK);
    config.ownship_source = source;
    config.execution_target = ExecutionTarget::Simulator;
    config.vehicle_class = VehicleClass::FixedWing;
    config.cruise_height_m = cruise_height_m;
    config.cruise_mps = CRUISE_IAS_MPS;
    config.limits.max_horizontal_mps = 65.0;
    config.limits.max_vertical_mps = 5.0;
    config.route_bank_deg = 22.0;
    config
}

/// North 6 km, then east 6 km, at `altitude_m`.
fn plan(altitude_m: f64) -> WaypointPlan {
    let plane = tangent_plane(anchor()).expect("anchor is plausible");
    WaypointPlan::new(
        [[0.0, 0.0], [6_000.0, 0.0], [6_000.0, 6_000.0]]
            .iter()
            .map(|[north, east]| {
                let p = plane.from_ned(&NedOffset::new(*north, *east, 0.0));
                PlannedWaypoint::new(
                    p.latitude_rad.to_degrees(),
                    p.longitude_rad.to_degrees(),
                    altitude_m,
                )
            })
            .collect(),
    )
}

/// What the navigator states at a time from the start of the flight.
#[derive(Clone, Copy)]
struct Statement {
    /// Its offset from the truth, north-east-down, metres.
    offset_ned_m: [f64; 3],
    /// Its 1-sigma horizontal and vertical uncertainty, metres.
    sigma_m: [f64; 2],
    quality: SolutionQuality,
}

/// The navigator's solution of `aircraft` as `statement` states it.
fn solve(
    aircraft: &Aircraft,
    now: MonotonicNanos,
    sequence: u32,
    statement: Statement,
) -> NavigationSolution {
    let plane = tangent_plane(anchor()).expect("anchor is plausible");
    let [north, east, down] = statement.offset_ned_m;
    let mut position = plane.from_ned(&NedOffset::new(
        aircraft.ned[0] + north,
        aircraft.ned[1] + east,
        aircraft.ned[2],
    ));
    // The simulated aircraft flies its altitude flat over the anchor's
    // plane, as its flight controller holds it; the plane's curvature
    // would otherwise add several metres over these legs.
    position.altitude_m = ALTITUDE_M - aircraft.ned[2] - down;
    let velocity = aircraft.ground_velocity();
    let composition = SourceComposition::of(SensorClass::Inertial);
    let [horizontal, vertical] = statement.sigma_m;
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
        SymmetricCov3::from_diagonal(horizontal.powi(2), horizontal.powi(2), vertical.powi(2)),
        SymmetricCov3::from_diagonal(0.25, 0.25, 0.25),
        IntegrityAssessment::new(
            statement.quality,
            composition,
            Redundancy::None,
            horizontal,
            vertical,
            FaultDetection::Unavailable,
        ),
        composition,
        Ellipsoid::WGS84,
    )
    .with_attitude(AttitudeQuaternion::new(half.cos(), 0.0, 0.0, half.sin()))
}

struct Flight {
    states: Vec<MissionState>,
    events: Vec<MissionEvent>,
}

/// Flies `engine` for `ticks` on the navigator `navigator` states at each
/// time, seconds. With `fc_height`, the flight controller's height above
/// the anchor's altitude goes to the engine each tick as its status.
fn fly(
    engine: &mut MissionEngine,
    ticks: usize,
    navigator: &dyn Fn(f64) -> Statement,
    fc_height: bool,
) -> Flight {
    let mut aircraft = Aircraft {
        ned: [-1_500.0, 0.0, 0.0],
        heading_rad: 0.0,
        bank_rad: 0.0,
        indicated_mps: CRUISE_IAS_MPS,
        wind: [0.0, 0.0],
        heading_bias_rad: 0.0,
    };
    let mut flight = Flight {
        states: Vec::new(),
        events: Vec::new(),
    };
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    for tick in 0..ticks {
        now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
        let elapsed_s = tick as f64 * STEP_S;
        let sequence = u32::try_from(tick).expect("a short flight");
        engine.on_onboard_solution(&solve(&aircraft, now, sequence, navigator(elapsed_s)), now);
        if fc_height {
            let mut status = VehicleStatus::ready(true, now);
            status.height_above_origin_m = Some(-aircraft.ned[2]);
            engine.on_vehicle_status(status);
        }
        let output = engine.tick(now);
        if let Some(action) = output.action {
            assert_eq!(action.action, ControlAction::Arm);
            engine.on_action_result(action.action_id, true);
        }
        flight.events.extend(output.events);
        flight.states.push(output.state);
        if let Some(intent) = output.intent.as_ref() {
            aircraft.fly(intent);
        }
        aircraft.step();
    }
    flight
}

/// A navigator with no position aid: Good while its horizontal
/// uncertainty is under 10 m, Degraded to 50 m, Unusable past it, growing
/// from 5 m to about 5 km over 250 s, and drifting east as it grows.
fn dead_reckoning_navigator(elapsed_s: f64) -> Statement {
    let horizontal = 5.0 + 0.08 * elapsed_s * elapsed_s;
    Statement {
        offset_ned_m: [0.0, 0.4 * elapsed_s, 0.0],
        sigma_m: [horizontal, 1.5],
        quality: if horizontal < 10.0 {
            SolutionQuality::Good
        } else if horizontal <= 50.0 {
            SolutionQuality::Degraded
        } else {
            SolutionQuality::Unusable
        },
    }
}

#[test]
fn a_navigator_without_a_position_aid_flies_every_leg_by_dead_reckoning() {
    let (mut engine, _) = MissionEngine::from_waypoints(
        &plan(ALTITUDE_M),
        config(OwnshipSource::onboard_navigation_gnss_denied(), 0.0),
    )
    .expect("builds");
    let flight = fly(&mut engine, 7_000, &dead_reckoning_navigator, false);
    assert_eq!(
        flight.states.last(),
        Some(&MissionState::Complete),
        "every waypoint sequenced on the navigator's own position"
    );
    let reports: Vec<f64> = flight
        .events
        .iter()
        .filter_map(|event| match event {
            MissionEvent::DeadReckoning {
                horizontal_1sigma_m,
                ..
            } => Some(*horizontal_1sigma_m),
            _ => None,
        })
        .collect();
    assert!(
        reports.first().is_some_and(|first| *first > 50.0),
        "the dead reckoning is reported when the limit is crossed: {reports:?}"
    );
    assert!(
        reports.windows(2).all(|pair| pair[1] >= 2.0 * pair[0])
            && reports.last().is_some_and(|last| *last > 1_000.0),
        "and again each time its uncertainty doubles, into kilometres: {reports:?}"
    );
    assert!(
        !flight
            .events
            .iter()
            .any(|event| matches!(event, MissionEvent::NavigationHold { .. })),
        "no hold while a solution is flown"
    );
    assert!(engine.counters().onboard_dead_reckoned > 0);
    assert_eq!(engine.counters().nav_source_mismatch, 0);
    let display = engine.navigation_display().dead_reckoning;
    assert!(
        display.is_some_and(|dead_reckoning| dead_reckoning.horizontal_1sigma_m > 1_000.0),
        "the display carries the dead reckoning and its uncertainty: {display:?}"
    );
    let steering = engine.navigation_display().steering;
    assert_eq!(steering.source, PositionSource::OnboardNavigation);
    assert!(
        steering.contributing.is_some_and(|classes| {
            classes.contains(SensorClass::Inertial) && !classes.contains(SensorClass::Gnss)
        }),
        "the display can tell this is not a satellite position: {steering:?}"
    );
    assert!(
        matches!(
            steering.integrity,
            SteeringIntegrity::DeadReckoning {
                quality: NavQuality::Unusable,
                horizontal_1sigma_m,
            } if horizontal_1sigma_m > 1_000.0
        ),
        "flown beyond its limits, with the navigator's own grade beside it: {steering:?}"
    );
}

#[test]
fn the_display_names_the_source_steered_on_and_why_there_is_none() {
    let (mut engine, _) = MissionEngine::from_waypoints(
        &plan(ALTITUDE_M),
        config(OwnshipSource::onboard_navigation(), 0.0),
    )
    .expect("builds");
    let before = engine.navigation_display();
    assert_eq!(
        before.phase,
        FlightPhase::Enroute,
        "no leg has a terminal role"
    );
    assert_eq!(before.steering.contributing, None);
    assert_eq!(
        before.steering.integrity,
        SteeringIntegrity::NoSolution {
            reason: StartWithheldReason::NavigationNotStarted
        }
    );

    fly(&mut engine, 200, &low_navigator(0.0), false);
    let flown = engine.navigation_display();
    assert_eq!(flown.phase, FlightPhase::Enroute);
    assert_eq!(flown.steering.source, PositionSource::OnboardNavigation);
    assert_eq!(
        flown.steering.integrity,
        SteeringIntegrity::WithinLimits {
            quality: NavQuality::Good,
            horizontal_1sigma_m: 5.0,
        }
    );

    // The navigator stops solving: its last solution ages past the limit.
    let later = MonotonicNanos::from_nanos(1_000_000_000 + 200 * STEP_NANOS + 5_000_000_000);
    engine.tick(later);
    let stale = engine.navigation_display().steering;
    assert_eq!(stale.contributing, None);
    assert_eq!(
        stale.integrity,
        SteeringIntegrity::NoSolution {
            reason: StartWithheldReason::NavigationStale
        }
    );
}

#[test]
fn the_same_navigator_holds_under_the_default_limits() {
    let (mut engine, _) = MissionEngine::from_waypoints(
        &plan(ALTITUDE_M),
        config(OwnshipSource::onboard_navigation(), 0.0),
    )
    .expect("builds");
    let flight = fly(&mut engine, 7_000, &dead_reckoning_navigator, false);
    assert_ne!(flight.states.last(), Some(&MissionState::Complete));
    assert!(flight.events.iter().any(|event| matches!(
        event,
        MissionEvent::NavigationHold {
            reason: StartWithheldReason::NavigationTooUncertain { .. }
                | StartWithheldReason::NavigationUnusable
        }
    )));
    assert!(engine.dead_reckoning().is_none());
    assert_eq!(engine.counters().nav_source_mismatch, 0);
    let steering = engine.navigation_display().steering;
    assert!(
        matches!(
            steering.integrity,
            SteeringIntegrity::NoSolution {
                reason: StartWithheldReason::NavigationTooUncertain { .. }
                    | StartWithheldReason::NavigationUnusable
            }
        ),
        "the heading held is not steered on a position: {steering:?}"
    );
}

/// A navigator whose altitude sits `low_m` under the truth, with a
/// barometric 1.5 m vertical uncertainty and a good horizontal fix.
fn low_navigator(low_m: f64) -> impl Fn(f64) -> Statement {
    move |_| Statement {
        offset_ned_m: [0.0, 0.0, low_m],
        sigma_m: [5.0, 1.5],
        quality: SolutionQuality::Good,
    }
}

/// The tick at which the mission left the climb, if it did.
fn climb_end(flight: &Flight) -> Option<usize> {
    let climb = flight
        .states
        .iter()
        .position(|state| *state == MissionState::Climb)?;
    flight.states[climb..]
        .iter()
        .position(|state| *state != MissionState::Climb)
        .map(|after| climb + after)
}

#[test]
fn a_climb_captures_with_the_navigators_altitude_five_metres_low() {
    let (mut engine, _) = MissionEngine::from_waypoints(
        &plan(ALTITUDE_M + 300.0),
        config(OwnshipSource::onboard_navigation_gnss_denied(), 300.0),
    )
    .expect("builds");
    let flight = fly(&mut engine, 4_000, &low_navigator(5.0), false);
    assert!(
        climb_end(&flight).is_some_and(|tick| flight.states[tick] == MissionState::Enroute),
        "the climb ends in the route"
    );
}

#[test]
fn a_climb_captures_on_the_flight_controllers_altitude_when_its_datum_is_known() {
    // Forty metres low is past the window: only the flight controller's
    // own altitude, on the origin datum the commands are turned by, can
    // say the climb is done.
    let mut with_datum = config(OwnshipSource::onboard_navigation_gnss_denied(), 300.0);
    with_datum.fc_origin_height_m = Some(ALTITUDE_M);
    let (mut engine, _) =
        MissionEngine::from_waypoints(&plan(ALTITUDE_M + 300.0), with_datum).expect("builds");
    let flight = fly(&mut engine, 4_000, &low_navigator(40.0), true);
    assert!(
        climb_end(&flight).is_some(),
        "captured on the FC's altitude"
    );

    let (mut engine, _) = MissionEngine::from_waypoints(
        &plan(ALTITUDE_M + 300.0),
        config(OwnshipSource::onboard_navigation_gnss_denied(), 300.0),
    )
    .expect("builds");
    let flight = fly(&mut engine, 4_000, &low_navigator(40.0), true);
    assert_eq!(
        climb_end(&flight),
        None,
        "without the datum the height alone says nothing"
    );
}
