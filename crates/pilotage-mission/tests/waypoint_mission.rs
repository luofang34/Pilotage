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
use pilotage_protocol::{ControlAction, ControlIntent};

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
    plan.capture_radius_m = 4.0;
    plan
}

#[test]
fn a_waypoint_list_flies_each_waypoint_at_its_own_altitude() {
    let plan = plan();
    let (mut engine, record) = MissionEngine::from_waypoints(&plan, config()).expect("builds");
    assert_eq!(record.idents, ["WP1", "CORNER", "WP3"]);
    assert_eq!(record.digest, plan.digest());
    let plane = tangent_plane(anchor()).expect("anchor is plausible");
    let (mut ned, mut velocity, mut yaw) = ([0.0_f64; 3], [0.0_f64; 3], 0.0_f64);
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let mut captured = Vec::new();
    let mut completed = false;
    for sequence in 0..20_000_u32 {
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
        for axis in 0..3 {
            ned[axis] += velocity[axis] * STEP_S;
        }
        for event in &output.events {
            if let MissionEvent::LegAdvanced { to_index, reason } = event {
                captured.push((*to_index, *reason, ANCHOR_ALT_M - ned[2]));
            }
        }
        if output.state == MissionState::Complete {
            completed = true;
            break;
        }
    }
    assert!(completed, "the plan completes");
    let to: Vec<usize> = captured.iter().map(|(index, _, _)| *index).collect();
    assert_eq!(to, [1, 2], "each leg advances once, in fly order");
    // The vehicle reaches each waypoint's own altitude before it moves on.
    for ((_, reason, altitude_m), expected_m) in captured.iter().zip([12.0, 18.0]) {
        assert!(matches!(reason, SequenceReason::Overflown), "{reason:?}");
        assert!(
            (altitude_m - ANCHOR_ALT_M - expected_m).abs() < 1.0,
            "{altitude_m} m at a waypoint planned at {expected_m} m above the anchor"
        );
    }
    let last = plane.from_ned(&navigate_geodesy::NedOffset::new(ned[0], ned[1], ned[2]));
    assert!((last.latitude_rad.to_degrees() - 47.0).abs() < 1e-4);
    assert!((last.longitude_rad.to_degrees() - 8.0008).abs() < 1e-4);
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
