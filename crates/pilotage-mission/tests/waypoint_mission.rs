#![cfg(test)]

//! A mission built from a waypoint list: exact positions, one altitude
//! for each waypoint, and the turn at each one. A kinematic truth model
//! flies the engine's own intents until the plan completes.

use navigate_contract::{ClockDomainId, GeodeticPosition, MonotonicNanos};
use navigate_geodesy::wgs84::tangent_plane;
use navigate_route::SequenceReason;
use pilotage_mission::{
    ExecutionTarget, MissionBuildError, MissionConfig, MissionEngine, MissionEvent, MissionState,
    OwnshipSample, OwnshipSource, PlannedWaypoint, TruthRole, WaypointPlan, WaypointTurn,
};
use pilotage_protocol::{ControlAction, ControlIntent, VehicleClass};

const CLOCK: ClockDomainId = ClockDomainId::new(7);
const STEP_NANOS: u64 = 50_000_000;
const STEP_S: f64 = 0.05;
const ANCHOR_ALT_M: f64 = 500.0;

fn anchor() -> GeodeticPosition {
    GeodeticPosition::new(47.0_f64.to_radians(), 8.0_f64.to_radians(), ANCHOR_ALT_M)
}

fn config() -> MissionConfig {
    let mut config = MissionConfig::new(String::new(), anchor(), CLOCK);
    config.ownship_source = OwnshipSource::SimulationTruth;
    config.execution_target = ExecutionTarget::Simulator;
    config.cruise_height_m = 10.0;
    config.cruise_mps = 6.0;
    config.limits.max_horizontal_mps = 6.0;
    config
}

/// Three waypoints, each at its own altitude; the middle one is flown
/// over.
fn plan() -> WaypointPlan {
    let mut corner = PlannedWaypoint::new(47.0006, 8.0008, ANCHOR_ALT_M + 18.0);
    corner.turn = WaypointTurn::FlyOver;
    corner.name = Some("CORNER".to_owned());
    let mut plan = WaypointPlan::new(vec![
        PlannedWaypoint::new(47.0006, 8.0, ANCHOR_ALT_M + 12.0),
        corner,
        PlannedWaypoint::new(47.0, 8.0008, ANCHOR_ALT_M + 8.0),
    ]);
    plan.capture_radius_m = Some(4.0);
    plan
}

/// One flight of a kinematic vehicle that follows the engine's intents.
struct Flight {
    /// Each leg advance: the leg's index, the reason, and the altitude.
    captured: Vec<(usize, SequenceReason, f64)>,
    /// The position in the anchor's NED plane when the mission completed.
    completed_at: Option<[f64; 3]>,
    /// The NED position and horizontal speed at each tick after the
    /// mission completed.
    after: Vec<([f64; 3], f64)>,
}

fn fly(engine: &mut MissionEngine, ticks_after_complete: usize) -> Flight {
    let (mut ned, mut velocity, mut yaw) = ([0.0_f64; 3], [0.0_f64; 3], 0.0_f64);
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let mut flight = Flight {
        captured: Vec::new(),
        completed_at: None,
        after: Vec::new(),
    };
    for sequence in 0..40_000_u32 {
        now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
        let sample = OwnshipSample {
            ned,
            ned_velocity: velocity,
            yaw_rad: Some(yaw),
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
        if let Some(ControlIntent::Velocity(v)) = output.intent.as_ref() {
            let (sin, cos) = yaw.sin_cos();
            let (vx, vy) = (f64::from(v.vx), f64::from(v.vy));
            velocity = [vx * cos - vy * sin, vx * sin + vy * cos, f64::from(v.vz)];
            yaw += f64::from(v.yaw_rate) * STEP_S;
        }
        for event in &output.events {
            if let MissionEvent::LegAdvanced { to_index, reason } = event {
                flight
                    .captured
                    .push((*to_index, *reason, ANCHOR_ALT_M - ned[2]));
            }
        }
        if output.state == MissionState::Complete {
            if flight.completed_at.is_none() {
                flight.completed_at = Some(ned);
            } else {
                flight.after.push((ned, velocity[0].hypot(velocity[1])));
            }
            if flight.after.len() >= ticks_after_complete {
                break;
            }
        }
        for axis in 0..3 {
            ned[axis] += velocity[axis] * STEP_S;
        }
    }
    flight
}

/// The offset of an anchor-plane NED position from the last waypoint:
/// horizontal and vertical distance, metres.
fn from_last_waypoint(plan: &WaypointPlan, ned: [f64; 3]) -> (f64, f64) {
    let plane = tangent_plane(anchor()).expect("anchor is plausible");
    let last = plan.waypoints.last().expect("a last waypoint");
    let target = plane.to_ned(&GeodeticPosition::new(
        last.latitude_deg.to_radians(),
        last.longitude_deg.to_radians(),
        last.altitude_m,
    ));
    (
        (ned[0] - target.north_m).hypot(ned[1] - target.east_m),
        (ned[2] - target.down_m).abs(),
    )
}

#[test]
fn a_waypoint_list_flies_each_waypoint_at_its_own_altitude() {
    let plan = plan();
    let (mut engine, record) = MissionEngine::from_waypoints(&plan, config()).expect("builds");
    assert_eq!(record.idents, ["WP1", "CORNER", "WP3"]);
    assert_eq!(record.digest, plan.digest());
    let flight = fly(&mut engine, 100);
    let to: Vec<usize> = flight.captured.iter().map(|(index, _, _)| *index).collect();
    assert_eq!(to, [1, 2], "each leg advances once, in fly order");
    // The vehicle reaches each waypoint's own altitude before it moves on.
    for ((_, reason, altitude_m), expected_m) in flight.captured.iter().zip([12.0, 18.0]) {
        assert!(matches!(reason, SequenceReason::Overflown), "{reason:?}");
        assert!(
            (altitude_m - ANCHOR_ALT_M - expected_m).abs() < 1.0,
            "{altitude_m} m at a waypoint planned at {expected_m} m above the anchor"
        );
    }
    // The plan completes as the vehicle passes the last waypoint, not a
    // capture radius short of it.
    let completed_at = flight.completed_at.expect("the plan completes");
    let (horizontal_m, vertical_m) = from_last_waypoint(&plan, completed_at);
    assert!(
        horizontal_m < 1.0,
        "{horizontal_m} m from the last waypoint"
    );
    assert!(
        vertical_m < 1.0,
        "{vertical_m} m above or below the last waypoint"
    );
    // A multicopter then stops.
    let (_, speed_mps) = flight.after.last().expect("ticks after completion");
    assert!(*speed_mps < 1e-6, "{speed_mps} m/s after completion");
}

#[test]
fn a_fixed_wing_orbits_the_last_waypoint_after_the_plan() {
    let mut plan = plan();
    plan.capture_radius_m = None;
    let mut config = config();
    config.vehicle_class = VehicleClass::FixedWing;
    let (mut engine, record) = MissionEngine::from_waypoints(&plan, config).expect("builds");
    let flight = fly(&mut engine, 1_200);
    let completed_at = flight.completed_at.expect("the plan completes");
    let (horizontal_m, _) = from_last_waypoint(&plan, completed_at);
    assert!(
        horizontal_m < 1.0,
        "{horizontal_m} m from the last waypoint"
    );
    // After a settling turn, the aircraft keeps its speed and stays on
    // the orbit at the capture radius around the last waypoint.
    let settled = flight.after.get(400..).expect("40 s of orbit");
    let radius_m = record.capture_radius_m;
    for (ned, speed_mps) in settled {
        let (distance_m, vertical_m) = from_last_waypoint(&plan, *ned);
        assert!(*speed_mps > 4.0, "{speed_mps} m/s in the orbit");
        assert!(
            (distance_m - radius_m).abs() < 0.1 * radius_m,
            "{distance_m} m from the centre of a {radius_m} m orbit"
        );
        assert!(vertical_m < 1.0, "{vertical_m} m off the orbit altitude");
    }
}

#[test]
fn a_waypoint_list_is_refused_with_its_defect() {
    let empty = MissionEngine::from_waypoints(&WaypointPlan::new(Vec::new()), config());
    assert!(matches!(empty, Err(MissionBuildError::EmptyWaypointList)));

    let mut off_earth = plan();
    off_earth.waypoints[1].latitude_deg = 91.0;
    assert!(matches!(
        MissionEngine::from_waypoints(&off_earth, config()),
        Err(MissionBuildError::WaypointPosition { index: 1, .. })
    ));

    let mut not_a_number = plan();
    not_a_number.waypoints[2].altitude_m = f64::NAN;
    assert!(matches!(
        MissionEngine::from_waypoints(&not_a_number, config()),
        Err(MissionBuildError::WaypointPosition { index: 2, .. })
    ));

    let mut long_name = plan();
    long_name.waypoints[0].name = Some("A".repeat(33));
    assert!(matches!(
        MissionEngine::from_waypoints(&long_name, config()),
        Err(MissionBuildError::NavigationIdent { .. })
    ));
}

#[test]
fn a_changed_waypoint_changes_the_plan_identity() {
    let (_, first) = MissionEngine::from_waypoints(&plan(), config()).expect("builds");
    let mut moved = plan();
    moved.waypoints[2].altitude_m += 1.0;
    let (_, second) = MissionEngine::from_waypoints(&moved, config()).expect("builds");
    assert_ne!(first.digest, second.digest);
    assert_ne!(
        first.mission_identity.content_digest,
        second.mission_identity.content_digest
    );
}

#[test]
fn the_capture_radius_follows_the_vehicle_unless_the_plan_sets_one() {
    // 6 m/s at a 25° bank turns on a radius of v² / (g tan 25°).
    let mut derived = plan();
    derived.capture_radius_m = None;
    let (_, record) = MissionEngine::from_waypoints(&derived, config()).expect("builds");
    let expected_m = 36.0 / (9.806_65 * 25.0_f64.to_radians().tan());
    assert!(
        (record.capture_radius_m - expected_m).abs() < 1e-3,
        "{}",
        record.capture_radius_m
    );
    let (_, record) = MissionEngine::from_waypoints(&plan(), config()).expect("builds");
    assert_eq!(record.capture_radius_m, 4.0);
    // A slow vehicle keeps the minimum radius.
    assert_eq!(pilotage_mission::capture_radius_for(1.0, 25.0), 2.0);
    // A bank outside 0° to 90° gives the minimum.
    assert_eq!(pilotage_mission::capture_radius_for(50.0, 90.0), 2.0);
    assert_eq!(pilotage_mission::capture_radius_for(50.0, 0.0), 2.0);
    let vehicle = WaypointPlan::for_vehicle(plan().waypoints, 10.0, 45.0);
    let radius_m = vehicle.capture_radius_m.expect("a radius");
    assert!((radius_m - 100.0 / 9.806_65).abs() < 1e-6, "{radius_m}");
}
