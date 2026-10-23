#![cfg(test)]

//! A line end the vehicle passes too far to the side to sequence it:
//! guidance turns back to it instead of following the leg's course away
//! from the area.

use navigate_contract::{ClockDomainId, GeodeticPosition, MonotonicNanos};
use navigate_geodesy::{LocalTangentPlane, NedOffset};
use pilotage_mission::{
    ExecutionTarget, MissionConfig, MissionEngine, MissionEvent, MissionState, OwnshipSample,
    OwnshipSource, SurveyRequest, TruthRole, plan_survey,
};
use pilotage_protocol::{ControlAction, ControlIntent};

const CLOCK: ClockDomainId = ClockDomainId::new(7);
const STEP_NANOS: u64 = 50_000_000;
const STEP_S: f64 = 0.05;
const HEIGHT_M: f64 = 15.0;
const CRUISE_MPS: f64 = 12.0;
/// How far the gust pushes the vehicle across the line, metres.
const GUST_M: f64 = 40.0;

fn anchor() -> GeodeticPosition {
    GeodeticPosition::new(47.0_f64.to_radians(), 8.0_f64.to_radians(), 500.0)
}

fn request() -> SurveyRequest {
    let plane = LocalTangentPlane::new(anchor()).expect("anchor is plausible");
    let polygon = [[0.0, 0.0], [300.0, 0.0], [300.0, 120.0], [0.0, 120.0]]
        .iter()
        .map(|[north, east]| {
            let p = plane.from_ned(&NedOffset::new(*north, *east, 0.0));
            [p.latitude_rad.to_degrees(), p.longitude_rad.to_degrees()]
        })
        .collect();
    let mut request = SurveyRequest::new(polygon);
    request.across_track_fov_rad = 1.5;
    request
}

fn config() -> MissionConfig {
    let mut config = MissionConfig::new(String::new(), anchor(), CLOCK);
    config.ownship_source = OwnshipSource::SimulationTruth;
    config.execution_target = ExecutionTarget::Simulator;
    config.cruise_height_m = HEIGHT_M;
    config.cruise_mps = CRUISE_MPS;
    config.limits.max_horizontal_mps = CRUISE_MPS;
    config
}

/// The first line's far end in the truth frame: north, east.
fn first_line_end() -> [f64; 2] {
    let plane = LocalTangentPlane::new(anchor()).expect("anchor is plausible");
    let plan = plan_survey(&request(), &anchor(), HEIGHT_M).expect("plans");
    let end = plane.to_ned(&plan.plan.waypoints[1].position);
    [end.north_m, end.east_m]
}

#[test]
fn a_line_end_passed_outside_the_capture_radius_is_flown_back_to() {
    let (mut engine, record) = MissionEngine::from_survey(&request(), config()).expect("builds");
    let end = first_line_end();
    let (mut ned, mut velocity, mut yaw) = ([0.0_f64; 3], [0.0_f64; 3], 0.0_f64);
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let (mut gusted, mut farthest, mut completed) = (false, 0.0_f64, false);
    let mut reports = Vec::new();
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
        for axis in 0..3 {
            ned[axis] += velocity[axis] * STEP_S;
        }
        let to_end = (ned[0] - end[0]).hypot(ned[1] - end[1]);
        if !gusted && to_end < 2.0 * GUST_M {
            // A gust across the line just before its end: the vehicle
            // arrives too far to the side to capture it.
            ned[1] -= GUST_M;
            gusted = true;
        }
        farthest = farthest.max(ned[0] - end[0]);
        reports.extend(output.events.iter().filter_map(|event| match event {
            MissionEvent::FixOverflown { to_index } => Some(*to_index),
            _ => None,
        }));
        if output.state == MissionState::Complete {
            completed = true;
            break;
        }
    }
    assert!(gusted, "the gust fired before the line end");
    assert!(completed, "the survey completes after the missed line end");
    assert!(
        engine.counters().fix_overflown > 0,
        "guidance flew back to the missed line end"
    );
    assert!(
        farthest < 2.0 * GUST_M,
        "the vehicle went {farthest} m past the line end"
    );
    assert!(!reports.is_empty(), "the miss is reported");
    assert!(
        reports.windows(2).all(|pair| pair[0] < pair[1]),
        "each missed fix reports once, in fly order: {reports:?}"
    );
    assert!(record.capture_radius_m < GUST_M);
}
