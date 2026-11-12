#![cfg(test)]

//! Fly-by and fly-over waypoints flown closed loop with Aviate's C172
//! lateral law at 60 m/s, on heading and on velocity demands, in still air
//! and in a 12 m/s wind across the first leg from either side. The first
//! leg runs north, and the second turns right from it by the case's angle.
//! A fly-by turn rolls out on the next leg without swinging past it, and
//! late on the next leg the aircraft is on it. A fly-over turn passes over
//! the waypoint, swings out, and intercepts the next leg without crossing
//! it.

#[path = "track_keeping/aircraft.rs"]
mod aircraft;

use aircraft::Aircraft;
use navigate_contract::{
    AttitudeQuaternion, ClockDomainId, Ellipsoid, FaultDetection, GeodeticPosition,
    IntegrityAssessment, MonotonicNanos, NavigationSolution, NedVelocity, Redundancy, SensorClass,
    SolutionQuality, SolutionStamp, SourceComposition, SourceEpoch, SymmetricCov3,
    WrappingSequence,
};
use navigate_geodesy::NedOffset;
use navigate_geodesy::wgs84::tangent_plane;
use pilotage_mission::{
    ExecutionTarget, MissionConfig, MissionEngine, MissionState, OwnshipSample, OwnshipSource,
    PlannedWaypoint, TruthRole, WaypointPlan, WaypointTurn,
};
use pilotage_protocol::{ControlAction, VehicleClass};

const CLOCK: ClockDomainId = ClockDomainId::new(7);
const STEP_NANOS: u64 = 50_000_000;
pub(crate) const STEP_S: f64 = 0.05;
pub(crate) const ALTITUDE_M: f64 = 2_000.0;
const FIRST_LEG_M: f64 = 6_000.0;
const SECOND_LEG_M: f64 = 10_000.0;

fn anchor() -> GeodeticPosition {
    GeodeticPosition::new(40.0_f64.to_radians(), -75.0_f64.to_radians(), ALTITUDE_M)
}

fn config(navigator: bool) -> MissionConfig {
    let mut config = MissionConfig::new(String::new(), anchor(), CLOCK);
    config.ownship_source = if navigator {
        OwnshipSource::onboard_navigation()
    } else {
        OwnshipSource::SimulationTruth
    };
    config.execution_target = ExecutionTarget::Simulator;
    config.vehicle_class = VehicleClass::FixedWing;
    config.cruise_height_m = 0.0;
    config.cruise_mps = 60.0;
    config.limits.max_horizontal_mps = 65.0;
    config.limits.max_vertical_mps = 5.0;
    config.limits.max_yaw_rate_rps = 0.2;
    config.route_bank_deg = 22.0;
    config
}

/// North, then right by `turn_deg` at a waypoint of `turn`.
fn plan(turn_deg: f64, turn: WaypointTurn) -> WaypointPlan {
    let plane = tangent_plane(anchor()).expect("anchor is plausible");
    let (sin, cos) = turn_deg.to_radians().sin_cos();
    let points = [
        [0.0, 0.0],
        [FIRST_LEG_M, 0.0],
        [FIRST_LEG_M + SECOND_LEG_M * cos, SECOND_LEG_M * sin],
    ];
    let mut waypoints: Vec<PlannedWaypoint> = points
        .iter()
        .map(|[north, east]| {
            let p = plane.from_ned(&NedOffset::new(*north, *east, 0.0));
            PlannedWaypoint::new(
                p.latitude_rad.to_degrees(),
                p.longitude_rad.to_degrees(),
                ALTITUDE_M,
            )
        })
        .collect();
    if let Some(corner) = waypoints.get_mut(1) {
        corner.turn = turn;
    }
    WaypointPlan::new(waypoints)
}

fn solution(aircraft: &Aircraft, now: MonotonicNanos, sequence: u32) -> NavigationSolution {
    const SIGMA_M: f64 = 5.0;
    let plane = tangent_plane(anchor()).expect("anchor is plausible");
    let position = plane.from_ned(&NedOffset::new(aircraft.ned[0], aircraft.ned[1], 0.0));
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
        NedVelocity::new(velocity[0], velocity[1], 0.0),
        SymmetricCov3::from_diagonal(25.0, 25.0, 25.0),
        SymmetricCov3::from_diagonal(0.25, 0.25, 0.25),
        IntegrityAssessment::new(
            SolutionQuality::Good,
            composition,
            Redundancy::None,
            SIGMA_M,
            SIGMA_M,
            FaultDetection::Unavailable,
        ),
        composition,
        Ellipsoid::WGS84,
    )
    .with_attitude(AttitudeQuaternion::new(half.cos(), 0.0, 0.0, half.sin()))
}

/// The track of one flight in the anchor's NED plane, until the plan
/// completes.
fn fly(navigator: bool, wind_mps: [f64; 2], plan: &WaypointPlan) -> Vec<[f64; 2]> {
    let (mut engine, _) = MissionEngine::from_waypoints(plan, config(navigator)).expect("builds");
    let mut aircraft = Aircraft::new([-1_000.0, 0.0], 0.0, wind_mps, 0.0);
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let mut track = Vec::new();
    for sequence in 0..30_000_u32 {
        now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
        if navigator {
            engine.on_onboard_solution(&solution(&aircraft, now, sequence), now);
        } else {
            let velocity = aircraft.ground_velocity();
            let sample = OwnshipSample {
                ned: [aircraft.ned[0], aircraft.ned[1], 0.0],
                ned_velocity: [velocity[0], velocity[1], 0.0],
                yaw_rad: Some(aircraft.heading_rad),
                role: TruthRole::SimulationTruth,
                acquired_at: now,
                sequence,
            };
            engine.on_ownship(&sample, now);
        }
        let output = engine.tick(now);
        if let Some(action) = output.action {
            assert_eq!(action.action, ControlAction::Arm);
            engine.on_action_result(action.action_id, true);
        }
        track.push(aircraft.ned);
        if output.state == MissionState::Complete {
            break;
        }
        aircraft.fly(output.intent.as_ref());
    }
    track
}

/// Where the track runs relative to the second leg once past the corner,
/// short of the leg's end: the farthest left of it and right of it, the
/// closest to the corner, and the farthest off it late on the leg, metres.
fn relative_to_second_leg(track: &[[f64; 2]], turn_deg: f64) -> [f64; 4] {
    let (sin, cos) = turn_deg.to_radians().sin_cos();
    let from_corner = |p: &[f64; 2]| (p[0] - FIRST_LEG_M).hypot(p[1]);
    // Past the corner: from the closest pass of the waypoint on.
    let corner = track
        .iter()
        .enumerate()
        .min_by(|a, b| from_corner(a.1).total_cmp(&from_corner(b.1)))
        .map_or(0, |(index, _)| index);
    let closest = track.get(corner).map_or(f64::INFINITY, from_corner);
    let past = track.get(corner..).unwrap_or_default();
    past.iter().fold([0.0, 0.0, closest, 0.0], |worst, p| {
        let (north, east) = (p[0] - FIRST_LEG_M, p[1]);
        let along = north * cos + east * sin;
        let right = -north * sin + east * cos;
        if along > SECOND_LEG_M - 1_500.0 {
            return worst;
        }
        let late = if along > 0.6 * SECOND_LEG_M {
            worst[3].max(right.abs())
        } else {
            worst[3]
        };
        [worst[0].max(-right), worst[1].max(right), worst[2], late]
    })
}

const WINDS: [[f64; 2]; 3] = [[0.0, 0.0], [0.0, 12.0], [0.0, -12.0]];

#[test]
fn a_fly_by_turn_rolls_out_on_the_next_leg() {
    let mut report = Vec::new();
    for turn_deg in [30.0, 60.0, 90.0] {
        for navigator in [true, false] {
            for wind in WINDS {
                let track = fly(navigator, wind, &plan(turn_deg, WaypointTurn::FlyBy));
                let [left, _, _, late] = relative_to_second_leg(&track, turn_deg);
                // Left of the next leg is outside a right turn.
                if left > 30.0 || late > 5.0 {
                    report.push(format!(
                        "{turn_deg}° navigator {navigator} wind {wind:?}: {left:.0} m outside, \
                         {late:.1} m off late"
                    ));
                }
            }
        }
    }
    assert!(report.is_empty(), "{}", report.join("\n"));
}

#[test]
fn a_fly_over_turn_passes_over_the_waypoint_and_intercepts_the_next_leg() {
    let mut report = Vec::new();
    for turn_deg in [30.0, 60.0, 90.0, 120.0, 150.0] {
        for navigator in [true, false] {
            for wind in WINDS {
                let track = fly(navigator, wind, &plan(turn_deg, WaypointTurn::FlyOver));
                let [left, right, closest, late] = relative_to_second_leg(&track, turn_deg);
                // The turn swings left of the next leg; crossing it to the
                // right is the intercept overshooting. Guidance closes the
                // last of the swing on a proportional tail, so late on the
                // leg a twentieth of the swing may be left.
                if right > 30.0 || closest > 30.0 || late > 5.0_f64.max(0.05 * left) {
                    report.push(format!(
                        "{turn_deg}° navigator {navigator} wind {wind:?}: {right:.0} m across, \
                         {closest:.0} m from the waypoint, {late:.1} m off late"
                    ));
                }
            }
        }
    }
    assert!(report.is_empty(), "{}", report.join("\n"));
}
