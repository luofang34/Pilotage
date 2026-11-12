#![cfg(test)]

//! The vertical mode a display reads through a climb: off before the arm
//! request, a climb to the cruise altitude from it, and that altitude
//! held once captured, with the climb rate commanded only while the mode
//! climbs.

use navigate_contract::{ClockDomainId, GeodeticPosition, MonotonicNanos};
use pilotage_mission::{
    ExecutionTarget, MissionConfig, MissionEngine, MissionState, OwnshipSample, OwnshipSource,
    PlannedWaypoint, TruthRole, VerticalMode, WaypointPlan,
};
use pilotage_protocol::{ControlAction, ControlIntent};

const CLOCK: ClockDomainId = ClockDomainId::new(7);
const STEP_NANOS: u64 = 50_000_000;
const STEP_S: f64 = 0.05;
const ANCHOR_ALTITUDE_M: f64 = 500.0;
const CRUISE_HEIGHT_M: f64 = 15.0;

fn engine() -> MissionEngine {
    let anchor = GeodeticPosition::new(
        47.0_f64.to_radians(),
        8.0_f64.to_radians(),
        ANCHOR_ALTITUDE_M,
    );
    let mut config = MissionConfig::new(String::new(), anchor, CLOCK);
    config.ownship_source = OwnshipSource::SimulationTruth;
    config.execution_target = ExecutionTarget::Simulator;
    config.cruise_height_m = CRUISE_HEIGHT_M;
    let cruise_m = ANCHOR_ALTITUDE_M + CRUISE_HEIGHT_M;
    let plan = WaypointPlan::new(vec![
        PlannedWaypoint::new(47.002, 8.0, cruise_m),
        PlannedWaypoint::new(47.004, 8.0, cruise_m),
    ]);
    MissionEngine::from_waypoints(&plan, config)
        .expect("builds")
        .0
}

#[test]
fn the_display_reads_off_then_a_climb_then_the_altitude_held() {
    let mut engine = engine();
    assert_eq!(engine.navigation_display().vertical.mode, VerticalMode::Off);
    assert_eq!(engine.navigation_display().vertical.target_m, None);

    let cruise_m = ANCHOR_ALTITUDE_M + CRUISE_HEIGHT_M;
    let (mut ned, mut velocity, mut yaw) = ([0.0_f64; 3], [0.0_f64; 3], 0.0_f64);
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let mut modes = Vec::new();
    for sequence in 0..4_000_u32 {
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
        let vertical = engine.navigation_display().vertical;
        let climbing_up = matches!(
            output.intent.as_ref(),
            Some(ControlIntent::Velocity(v)) if v.vz < 0.0 && v.vx == 0.0 && v.vy == 0.0
        );
        if output.state == MissionState::Climb {
            assert_eq!(vertical.target_m, Some(cruise_m), "the climb's target");
            assert_eq!(
                climbing_up,
                vertical.mode == VerticalMode::Climb,
                "the climb rate is commanded exactly while the mode climbs"
            );
        }
        if modes.last() != Some(&vertical.mode) {
            modes.push(vertical.mode);
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
        if output.state == MissionState::Enroute && vertical.mode == VerticalMode::Hold {
            break;
        }
    }
    assert_eq!(
        modes,
        [VerticalMode::Climb, VerticalMode::Hold],
        "a climb from the arm request, then the altitude held"
    );
    assert_eq!(
        engine.navigation_display().vertical.target_m,
        Some(cruise_m)
    );
    assert_eq!(engine.counters().vertical_mode_mismatch, 0);
}
