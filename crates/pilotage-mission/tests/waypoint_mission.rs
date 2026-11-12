#![cfg(test)]

//! A mission built from a waypoint list: exact positions, one altitude
//! for each waypoint, and the turn at each one. A kinematic truth model
//! flies the engine's own intents until the plan completes.

use core::f64::consts::PI;
use navigate_contract::{ClockDomainId, GeodeticPosition, MonotonicNanos};
use navigate_geodesy::wgs84::tangent_plane;
use navigate_route::SequenceReason;

use pilotage_mission::holding_pattern::STANDARD_RATE_RPS;
use pilotage_mission::{
    ExecutionTarget, HoldingEntry, HoldingPattern, HoldingTurn, MissionBuildError, MissionConfig,
    MissionEngine, MissionEvent, MissionState, OwnshipSample, OwnshipSource, PlannedWaypoint,
    TruthRole, WaypointPlan, WaypointTurn, true_airspeed_for,
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
    /// The events at the end of the plan.
    end_events: Vec<MissionEvent>,
}

/// How the simulated vehicle answers a velocity intent.
#[derive(Clone, Copy, PartialEq)]
enum Plant {
    /// It flies the body-frame velocity as asked, over the ground, and
    /// turns at the commanded yaw rate.
    Kinematic,
    /// A fixed wing's flight controller in still air: the horizontal
    /// direction is the course over the ground, the horizontal speed the
    /// indicated airspeed, and the heading is the course (no sideslip).
    FixedWing,
}

fn fly(engine: &mut MissionEngine, ticks_after_complete: usize) -> Flight {
    fly_as(engine, ticks_after_complete, Plant::Kinematic)
}

fn fly_as(engine: &mut MissionEngine, ticks_after_complete: usize, plant: Plant) -> Flight {
    let (mut ned, mut velocity, mut yaw) = ([0.0_f64; 3], [0.0_f64; 3], 0.0_f64);
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let mut flight = Flight {
        captured: Vec::new(),
        completed_at: None,
        after: Vec::new(),
        end_events: Vec::new(),
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
            match plant {
                Plant::Kinematic => yaw += f64::from(v.yaw_rate) * STEP_S,
                Plant::FixedWing => {
                    let indicated = velocity[0].hypot(velocity[1]);
                    if indicated > 0.0 {
                        yaw = velocity[1].atan2(velocity[0]);
                        let ground = true_airspeed_for(indicated, ANCHOR_ALT_M - ned[2]);
                        velocity[0] *= ground / indicated;
                        velocity[1] *= ground / indicated;
                    }
                }
            }
        }
        for event in &output.events {
            match event {
                MissionEvent::LegAdvanced { to_index, reason } => {
                    flight
                        .captured
                        .push((*to_index, *reason, ANCHOR_ALT_M - ned[2]));
                }
                MissionEvent::LastWaypointSequenced { .. }
                | MissionEvent::HoldingPatternStarted { .. }
                | MissionEvent::HoldingPatternEntered { .. } => {
                    flight.end_events.push(event.clone());
                }
                _ => {}
            }
        }
        if matches!(
            output.state,
            MissionState::Complete | MissionState::AfterPlan
        ) {
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
    // A multicopter flies on after the last waypoint sequences and
    // completes the plan at it, within the arrival radius, not a capture
    // radius short of it.
    let completed_at = flight.completed_at.expect("the plan completes");
    let (horizontal_m, vertical_m) = from_last_waypoint(&plan, completed_at);
    let limit_m = plan.arrival_radius_m + 0.1;
    assert!(
        horizontal_m <= limit_m,
        "{horizontal_m} m from the last waypoint"
    );
    assert!(
        vertical_m <= limit_m,
        "{vertical_m} m off the last waypoint"
    );
    assert!(matches!(
        flight.end_events.as_slice(),
        [MissionEvent::LastWaypointSequenced { .. }]
    ));
    // It then stops.
    let (_, speed_mps) = flight.after.last().expect("ticks after completion");
    assert!(*speed_mps < 1e-6, "{speed_mps} m/s after completion");
}

/// The offset of an anchor-plane NED position from the last waypoint in
/// the frame of a pattern on the plan's final course, due south: along
/// the inbound course, and toward the holding side of `turn`.
fn in_pattern(plan: &WaypointPlan, ned: [f64; 3], turn: HoldingTurn) -> [f64; 2] {
    let plane = tangent_plane(anchor()).expect("anchor is plausible");
    let last = plan.waypoints.last().expect("a last waypoint");
    let fix = plane.to_ned(&GeodeticPosition::new(
        last.latitude_deg.to_radians(),
        last.longitude_deg.to_radians(),
        last.altitude_m,
    ));
    // Right of a southbound course is west.
    let right = -(ned[1] - fix.east_m);
    let side = match turn {
        HoldingTurn::Right => right,
        HoldingTurn::Left => -right,
    };
    [-(ned[0] - fix.north_m), side]
}

/// The greatest reach of the positions after the first `skip` ticks past
/// the racetrack of `pattern`, metres: its own legs and turns, which it
/// sizes at the true airspeed a fixed wing flies.
fn reach_outside(
    plan: &WaypointPlan,
    flight: &Flight,
    pattern: &HoldingPattern,
    skip: usize,
) -> f64 {
    let radius = pattern.turn_radius_m;
    let length = pattern.leg_length_m;
    flight
        .after
        .iter()
        .skip(skip)
        .map(|(ned, _)| {
            let [x, y] = in_pattern(plan, *ned, pattern.turn);
            (-length - radius - x)
                .max(x - radius)
                .max(-y)
                .max(y - 2.0 * radius)
        })
        .fold(f64::NEG_INFINITY, f64::max)
}

fn started_pattern(flight: &Flight) -> HoldingPattern {
    flight
        .end_events
        .iter()
        .find_map(|event| match event {
            MissionEvent::HoldingPatternStarted { pattern } => Some(pattern.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no holding pattern: {:?}", flight.end_events))
}

#[test]
fn a_fixed_wing_holds_at_the_last_waypoint_on_the_final_course_after_the_plan() {
    let mut plan = plan();
    plan.capture_radius_m = None;
    let mut config = config();
    config.vehicle_class = VehicleClass::FixedWing;
    let (mut engine, record) = MissionEngine::from_waypoints(&plan, config).expect("builds");
    let flight = fly_as(&mut engine, 6_000, Plant::FixedWing);
    // The plan completes when the last waypoint sequences: the aircraft
    // cannot stop at it.
    let completed_at = flight.completed_at.expect("the plan completes");
    let (horizontal_m, _) = from_last_waypoint(&plan, completed_at);
    assert!(
        horizontal_m <= record.capture_radius_m + 0.5,
        "{horizontal_m} m"
    );
    let Some(MissionEvent::LastWaypointSequenced { distance_m, .. }) = flight.end_events.first()
    else {
        panic!("{:?}", flight.end_events);
    };
    assert!((distance_m - horizontal_m).abs() < 0.5, "{distance_m} m");
    // The pattern: at the last waypoint and its altitude, inbound on the
    // final leg's course, right turns, one-minute legs, standard rate.
    let pattern = started_pattern(&flight);
    let last = plan.waypoints.last().expect("a last waypoint");
    assert_eq!(pattern.fix_ident, "WP3");
    assert_eq!(pattern.fix.altitude_m, last.altitude_m);
    assert!(
        (pattern.inbound_course_rad - PI).abs() < 1e-3,
        "{}",
        pattern.inbound_course_rad
    );
    assert_eq!(pattern.turn, HoldingTurn::Right);
    assert_eq!(pattern.leg_time_s, 60.0);
    let airspeed = true_airspeed_for(6.0, last.altitude_m);
    assert!((pattern.turn_radius_m - airspeed / STANDARD_RATE_RPS).abs() < 1e-6);
    // Arriving on the inbound course is a direct entry.
    assert!(
        flight
            .end_events
            .contains(&MissionEvent::HoldingPatternEntered {
                entry: HoldingEntry::Direct
            }),
        "{:?}",
        flight.end_events
    );
    // The aircraft keeps its speed and its altitude, and flies the
    // racetrack on the holding side.
    for (ned, speed_mps) in &flight.after {
        let (_, vertical_m) = from_last_waypoint(&plan, *ned);
        assert!(*speed_mps > 5.9, "{speed_mps} m/s in the pattern");
        assert!(vertical_m < 1.0, "{vertical_m} m off the holding altitude");
    }
    let reach = reach_outside(&plan, &flight, &pattern, 0);
    assert!(reach < 0.15 * pattern.turn_radius_m, "{reach} m outside");
    let status = engine.holding_pattern().expect("a holding pattern");
    assert!(status.circuits >= 1, "{status:?}");
}

#[test]
fn a_configured_holding_pattern_overrides_the_default() {
    let mut plan = plan();
    plan.capture_radius_m = None;
    let mut config = config();
    config.vehicle_class = VehicleClass::FixedWing;
    config.terminal_holding_pattern.altitude_m = Some(ANCHOR_ALT_M + 25.0);
    config.terminal_holding_pattern.turn = HoldingTurn::Left;
    config.terminal_holding_pattern.leg_time_s = Some(30.0);
    let (mut engine, _) = MissionEngine::from_waypoints(&plan, config).expect("builds");
    let flight = fly_as(&mut engine, 6_000, Plant::FixedWing);
    let pattern = started_pattern(&flight);
    assert_eq!(pattern.fix.altitude_m, ANCHOR_ALT_M + 25.0);
    assert_eq!(pattern.turn, HoldingTurn::Left);
    assert_eq!(pattern.leg_time_s, 30.0);
    // The climb to the holding altitude at 6 m/s takes the first turn.
    let reach = reach_outside(&plan, &flight, &pattern, 1_200);
    assert!(reach < 0.15 * pattern.turn_radius_m, "{reach} m outside");
    let (ned, _) = flight.after.last().expect("ticks after completion");
    assert!(
        (-ned[2] - 25.0).abs() < 1.0,
        "{} m above the anchor",
        -ned[2]
    );
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
    // 6 m/s for the default 2 s.
    let mut derived = plan();
    derived.capture_radius_m = None;
    let (_, record) = MissionEngine::from_waypoints(&derived, config()).expect("builds");
    assert!(
        (record.capture_radius_m - 12.0).abs() < 1e-9,
        "{}",
        record.capture_radius_m
    );
    // A C172 at 55 m/s, on legs of some kilometres.
    let mut fast = config();
    fast.cruise_mps = 55.0;
    fast.limits.max_horizontal_mps = 55.0;
    let long = WaypointPlan::new(vec![
        PlannedWaypoint::new(47.03, 8.0, ANCHOR_ALT_M + 300.0),
        PlannedWaypoint::new(47.03, 8.05, ANCHOR_ALT_M + 300.0),
    ]);
    let (_, record) = MissionEngine::from_waypoints(&long, fast).expect("builds");
    assert!(
        (record.capture_radius_m - 110.0).abs() < 1e-9,
        "{}",
        record.capture_radius_m
    );
    // The plan's own radius wins.
    let (_, record) = MissionEngine::from_waypoints(&plan(), config()).expect("builds");
    assert_eq!(record.capture_radius_m, 4.0);
    // A slow vehicle keeps the minimum radius.
    assert_eq!(pilotage_mission::capture_radius_for(0.5, 2.0), 2.0);
    // The turn radius: v² / (g tan φ), never below its floor.
    let turn_m = pilotage_mission::turn_radius_for(10.0, 45.0, 2.0);
    assert!((turn_m - 100.0 / 9.806_65).abs() < 1e-6, "{turn_m}");
    assert_eq!(pilotage_mission::turn_radius_for(50.0, 90.0, 110.0), 110.0);
    assert_eq!(pilotage_mission::turn_radius_for(1.0, 25.0, 12.0), 12.0);
}

#[test]
fn a_multicopter_past_the_first_waypoint_joins_the_first_leg_ahead() {
    // The vehicle starts at the anchor, 50 m north of the first waypoint,
    // on the first leg's line toward the second.
    let plan = WaypointPlan::new(vec![
        PlannedWaypoint::new(46.99955, 8.0, ANCHOR_ALT_M + 10.0),
        PlannedWaypoint::new(47.0009, 8.0, ANCHOR_ALT_M + 10.0),
        PlannedWaypoint::new(47.0009, 8.0008, ANCHOR_ALT_M + 10.0),
    ]);
    let (mut engine, _) = MissionEngine::from_waypoints(&plan, config()).expect("builds");
    let flight = fly(&mut engine, 10);
    assert!(
        matches!(
            flight.captured.first(),
            Some((1, SequenceReason::Passed { .. }, _))
        ),
        "the first waypoint is passed at the start: {:?}",
        flight.captured
    );
    let to: Vec<usize> = flight.captured.iter().map(|(index, _, _)| *index).collect();
    assert_eq!(to, [1, 2], "the first waypoint is never flown back to");
    assert!(flight.completed_at.is_some(), "the plan completes");
}
