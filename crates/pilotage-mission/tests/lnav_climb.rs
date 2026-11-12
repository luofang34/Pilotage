#![cfg(test)]

//! A fixed wing climbs along its route, as an FMS climbs: through a
//! two-minute climb it turns onto the plan, sequences its first fix, and
//! holds the first leg, instead of holding the heading it started on for
//! kilometres off the plan. Both ways the engine steers a fixed wing are
//! flown: by velocity on the vehicle's estimate, and by heading and
//! altitude on the onboard navigator.

use navigate_contract::{
    AttitudeQuaternion, ClockDomainId, Ellipsoid, FaultDetection, GeodeticPosition,
    IntegrityAssessment, MonotonicNanos, NavigationSolution, NedVelocity, Redundancy, SensorClass,
    SolutionQuality, SolutionStamp, SourceComposition, SourceEpoch, SymmetricCov3,
    WrappingSequence,
};
use navigate_geodesy::NedOffset;
use navigate_geodesy::wgs84::tangent_plane;
use pilotage_mission::{
    ExecutionTarget, MissionConfig, MissionEngine, MissionEvent, MissionState, OwnshipSample,
    OwnshipSource, PlannedWaypoint, TruthRole, WaypointPlan, true_airspeed_for,
};
use pilotage_protocol::{ControlAction, ControlIntent, VehicleClass};

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
/// 300 m at 2.5 m/s: a two-minute climb.
const CLIMB_M: f64 = 300.0;
const CLIMB_RATE_MPS: f64 = 2.5;
/// North 2 km, north 12 km, then east 5 km.
const POINTS: [[f64; 2]; 3] = [[2_000.0, 0.0], [12_000.0, 0.0], [12_000.0, 5_000.0]];
/// The cross-track distance a fixed wing may swing to while it turns
/// onto the first leg at its first fix, metres: a direct-to leg has no
/// turn ahead of its fix, so the join overshoots by about a turn radius.
const JOIN_XTK_BOUND_M: f64 = 250.0;
/// The cross-track distance it holds once it has joined, metres, from
/// [`JOINED_AFTER_S`] past the first fix.
const XTK_BOUND_M: f64 = 50.0;
const JOINED_AFTER_S: f64 = 30.0;

fn anchor() -> GeodeticPosition {
    GeodeticPosition::new(40.0_f64.to_radians(), -75.0_f64.to_radians(), ALTITUDE_M)
}

fn config(source: OwnshipSource) -> MissionConfig {
    let mut config = MissionConfig::new(String::new(), anchor(), CLOCK);
    config.ownship_source = source;
    config.execution_target = ExecutionTarget::Simulator;
    config.vehicle_class = VehicleClass::FixedWing;
    config.cruise_height_m = CLIMB_M;
    config.climb_rate_mps = CLIMB_RATE_MPS;
    config.cruise_mps = CRUISE_IAS_MPS;
    config.limits.max_horizontal_mps = 65.0;
    config.limits.max_vertical_mps = 5.0;
    config.limits.max_yaw_rate_rps = 0.2;
    config.route_bank_deg = 22.0;
    config
}

fn plan() -> WaypointPlan {
    let plane = tangent_plane(anchor()).expect("anchor is plausible");
    WaypointPlan::new(
        POINTS
            .iter()
            .map(|[north, east]| {
                let p = plane.from_ned(&NedOffset::new(*north, *east, 0.0));
                PlannedWaypoint::new(
                    p.latitude_rad.to_degrees(),
                    p.longitude_rad.to_degrees(),
                    ALTITUDE_M + CLIMB_M,
                )
            })
            .collect(),
    )
}

/// The onboard navigator's exact solution of `aircraft`.
fn solve(aircraft: &Aircraft, now: MonotonicNanos, sequence: u32) -> NavigationSolution {
    let plane = tangent_plane(anchor()).expect("anchor is plausible");
    let mut position = plane.from_ned(&NedOffset::new(aircraft.ned[0], aircraft.ned[1], 0.0));
    position.altitude_m = ALTITUDE_M - aircraft.ned[2];
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
        SymmetricCov3::from_diagonal(25.0, 25.0, 4.0),
        SymmetricCov3::from_diagonal(0.25, 0.25, 0.25),
        IntegrityAssessment::new(
            SolutionQuality::Good,
            composition,
            Redundancy::None,
            5.0,
            2.0,
            FaultDetection::Unavailable,
        ),
        composition,
        Ellipsoid::WGS84,
    )
    .with_attitude(AttitudeQuaternion::new(half.cos(), 0.0, 0.0, half.sin()))
}

/// Flies a velocity intent the way a fixed wing's flight controller
/// reads it: its horizontal direction as a course, its magnitude as an
/// indicated airspeed, its vertical rate as a climb; no direction holds
/// the course.
fn fly_velocity(aircraft: &mut Aircraft, intent: &ControlIntent) {
    let ControlIntent::Velocity(v) = intent else {
        panic!("a fixed wing on its own estimate flies velocities: {intent:?}");
    };
    let (sin, cos) = aircraft.heading_rad.sin_cos();
    let (vx, vy) = (f64::from(v.vx), f64::from(v.vy));
    let [north, east] = [vx * cos - vy * sin, vx * sin + vy * cos];
    if north.hypot(east) > 0.0 {
        let error = east.atan2(north) - aircraft.heading_rad;
        let error = error.sin().atan2(error.cos());
        let airspeed = true_airspeed_for(aircraft.indicated_mps, ALTITUDE_M - aircraft.ned[2]);
        let max_rate = GRAVITY_MPS2 * ROUTE_BANK_RAD.tan() / airspeed;
        aircraft.heading_rad +=
            (error / COURSE_TIME_CONSTANT_S).clamp(-max_rate, max_rate) * STEP_S;
        aircraft.indicated_mps = north.hypot(east).min(65.0);
    }
    aircraft.ned[2] += f64::from(v.vz) * STEP_S;
}

struct Flight {
    /// The north-east position and the mission state at each tick.
    track: Vec<([f64; 2], MissionState)>,
    /// The state when each leg was made active, and the leg.
    advances: Vec<(usize, MissionState)>,
    held: bool,
}

/// Flies the plan from the anchor, heading east, off the route, with the
/// engine steering by velocity on the aircraft's own estimate or by
/// heading on the onboard navigator.
fn fly(by_heading: bool) -> Flight {
    let source = if by_heading {
        OwnshipSource::onboard_navigation()
    } else {
        OwnshipSource::vehicle_estimate()
    };
    let config = config(if by_heading {
        source
    } else {
        OwnshipSource::SimulationTruth
    });
    let (mut engine, _) = MissionEngine::from_waypoints(&plan(), config).expect("builds");
    let mut aircraft = Aircraft {
        ned: [0.0, 0.0, 0.0],
        heading_rad: core::f64::consts::FRAC_PI_2,
        bank_rad: 0.0,
        indicated_mps: CRUISE_IAS_MPS,
        wind: [0.0, 0.0],
        heading_bias_rad: 0.0,
    };
    let mut flight = Flight {
        track: Vec::new(),
        advances: Vec::new(),
        held: false,
    };
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    for tick in 0..12_000_u32 {
        now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
        if by_heading {
            engine.on_onboard_solution(&solve(&aircraft, now, tick), now);
        } else {
            let velocity = aircraft.ground_velocity();
            engine.on_ownship(
                &OwnshipSample {
                    ned: aircraft.ned,
                    ned_velocity: velocity,
                    yaw_rad: Some(aircraft.heading_rad),
                    role: TruthRole::SimulationTruth,
                    acquired_at: now,
                    sequence: tick,
                },
                now,
            );
        }
        let output = engine.tick(now);
        if let Some(action) = output.action {
            assert_eq!(action.action, ControlAction::Arm);
            engine.on_action_result(action.action_id, true);
        }
        for event in &output.events {
            match event {
                MissionEvent::LegAdvanced { to_index, .. } => {
                    flight.advances.push((*to_index, output.state));
                }
                MissionEvent::HoldingPatternStarted { .. } => flight.held = true,
                _ => {}
            }
        }
        flight
            .track
            .push(([aircraft.ned[0], aircraft.ned[1]], output.state));
        match output.intent.as_ref() {
            Some(intent) if by_heading => aircraft.fly(intent),
            Some(intent) => fly_velocity(&mut aircraft, intent),
            None => {}
        }
        aircraft.step();
        if flight.held {
            break;
        }
    }
    assert_eq!(engine.counters().vertical_mode_mismatch, 0);
    assert_eq!(engine.counters().lifecycle_mismatch, 0);
    flight
}

fn assert_climbs_along_the_route(flight: &Flight) {
    // The first fix is sequenced in the climb, not after it.
    assert!(
        matches!(flight.advances.first(), Some((1, MissionState::Climb))),
        "the first fix is sequenced during the climb: {:?}",
        flight.advances
    );
    // From the first fix to the top of the climb the aircraft holds the
    // first leg, north along the east = 0 line: within a turn's swing
    // while it joins, and close on it after.
    let on_leg: Vec<f64> = flight
        .track
        .iter()
        .skip_while(|(ned, _)| ned[0] < POINTS[0][0])
        .take_while(|(_, state)| *state == MissionState::Climb)
        .map(|(ned, _)| ned[1].abs())
        .collect();
    let joined_after = (JOINED_AFTER_S / STEP_S) as usize;
    assert!(
        on_leg.len() > joined_after,
        "the climb went on {} s past the first fix",
        on_leg.len() as f64 * STEP_S
    );
    let worst = |xtk: &[f64]| xtk.iter().copied().fold(0.0_f64, f64::max);
    let join = worst(&on_leg);
    let held = worst(on_leg.get(joined_after..).unwrap_or_default());
    assert!(
        join < JOIN_XTK_BOUND_M && held < XTK_BOUND_M,
        "the climb ran {join:.0} m off the first leg while joining it and {held:.0} m after"
    );
    assert!(
        flight.held,
        "the hold at the last fix is reached: {:?}",
        flight.advances
    );
}

#[test]
fn a_fixed_wing_steered_by_velocity_climbs_along_its_route() {
    assert_climbs_along_the_route(&fly(false));
}

#[test]
fn a_fixed_wing_steered_by_heading_climbs_along_its_route() {
    assert_climbs_along_the_route(&fly(true));
}
