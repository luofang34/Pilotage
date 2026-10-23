#![cfg(test)]

//! A fast survey with a vehicle that lags its commands, as a multicopter
//! does: each line end turns onto the next line without flying past a
//! fix, and the turns stay outside the survey area.

use navigate_contract::{ClockDomainId, GeodeticPosition, MonotonicNanos};
use navigate_geodesy::{LocalTangentPlane, NedOffset};
use pilotage_mission::{
    ExecutionTarget, MissionConfig, MissionEngine, MissionState, OwnshipSample, OwnshipSource,
    SurveyRequest, TruthRole,
};
use pilotage_protocol::{ControlAction, ControlIntent};

const CLOCK: ClockDomainId = ClockDomainId::new(7);
const STEP_NANOS: u64 = 20_000_000;
const STEP_S: f64 = 0.02;
const HEIGHT_M: f64 = 110.0;
const CRUISE_MPS: f64 = 12.0;
/// The simulated multicopter's first-order velocity and yaw-rate response.
const RESPONSE_S: f64 = 0.5;
const YAW_RATE_LIMIT_RPS: f64 = 1.745;
/// Commands reach the vehicle this many steps after the sample they
/// answer: the host's control loop and the link.
const LATENCY_STEPS: usize = 5;
const AREA_NORTH_M: f64 = 400.0;
const AREA_EAST_M: f64 = 400.0;

fn anchor() -> GeodeticPosition {
    GeodeticPosition::new(40.34_f64.to_radians(), -74.66_f64.to_radians(), 40.0)
}

fn request() -> SurveyRequest {
    let plane = LocalTangentPlane::new(anchor()).expect("anchor is plausible");
    let polygon = [
        [0.0, 0.0],
        [AREA_NORTH_M, 0.0],
        [AREA_NORTH_M, AREA_EAST_M],
        [0.0, AREA_EAST_M],
    ]
    .iter()
    .map(|[north, east]| {
        let p = plane.from_ned(&NedOffset::new(*north, *east, 0.0));
        [p.latitude_rad.to_degrees(), p.longitude_rad.to_degrees()]
    })
    .collect();
    let mut request = SurveyRequest::new(polygon);
    request.line_bearing_rad = Some(0.0);
    request.fit_to_cruise(CRUISE_MPS);
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

/// A point mass whose velocity and yaw rate follow the command with a
/// first-order lag, commanded in the body frame.
#[derive(Default)]
struct Vehicle {
    ned: [f64; 3],
    velocity: [f64; 3],
    yaw: f64,
    yaw_rate: f64,
}

impl Vehicle {
    fn step(&mut self, command: Option<&ControlIntent>) {
        let gain = STEP_S / (RESPONSE_S + STEP_S);
        let (mut target, mut yaw_rate) = ([0.0; 3], 0.0);
        if let Some(ControlIntent::Velocity(v)) = command {
            let (sin, cos) = self.yaw.sin_cos();
            let (vx, vy) = (f64::from(v.vx), f64::from(v.vy));
            target = [vx * cos - vy * sin, vx * sin + vy * cos, f64::from(v.vz)];
            yaw_rate = f64::from(v.yaw_rate).clamp(-YAW_RATE_LIMIT_RPS, YAW_RATE_LIMIT_RPS);
        }
        for ((velocity, position), wanted) in
            self.velocity.iter_mut().zip(&mut self.ned).zip(target)
        {
            *velocity += (wanted - *velocity) * gain;
            *position += *velocity * STEP_S;
        }
        self.yaw_rate += (yaw_rate - self.yaw_rate) * gain;
        self.yaw += self.yaw_rate * STEP_S;
    }
}

#[test]
fn a_fast_survey_turns_at_each_line_end_without_flying_past_a_fix() {
    let (mut engine, record) = MissionEngine::from_survey(&request(), config()).expect("builds");
    assert!(record.line_count >= 5, "{} lines", record.line_count);
    let mut vehicle = Vehicle::default();
    let mut commands: std::collections::VecDeque<Option<ControlIntent>> =
        std::iter::repeat_n(None, LATENCY_STEPS).collect();
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let (mut completed, mut inside_turn_m) = (false, 0.0_f64);
    for sequence in 0..100_000_u32 {
        now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
        let sample = OwnshipSample {
            ned: vehicle.ned,
            ned_velocity: vehicle.velocity,
            yaw_rad: Some(vehicle.yaw),
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
        commands.push_back(output.intent);
        vehicle.step(commands.pop_front().flatten().as_ref());
        // A turn is flown where the course leaves north or south; it
        // belongs outside the area's north and south edges.
        let [vn, ve, _] = vehicle.velocity;
        let turning = ve.abs() > 0.5 * vn.abs() && vn.hypot(ve) > 2.0;
        let north = vehicle.ned[0];
        if turning && north > 0.0 && north < AREA_NORTH_M && vehicle.ned[2] < -HEIGHT_M / 2.0 {
            inside_turn_m = inside_turn_m.max(north.min(AREA_NORTH_M - north));
        }
        if output.state == MissionState::Complete {
            completed = true;
            break;
        }
    }
    assert!(completed, "the survey completes");
    assert_eq!(
        engine.counters().fix_overflown,
        0,
        "every fix sequenced on the first pass"
    );
    assert!(
        inside_turn_m < 5.0,
        "a turn reached {inside_turn_m} m into the area"
    );
}
