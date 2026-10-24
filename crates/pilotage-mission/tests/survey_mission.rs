#![cfg(test)]

//! A survey mission flown closed-loop: the engine built from an area
//! request arms, climbs, flies every line end in order, and completes.

use navigate_contract::{ClockDomainId, GeodeticPosition, MonotonicNanos};
use navigate_fpl::SequenceReason;
use navigate_geodesy::{LocalTangentPlane, NedOffset};
use pilotage_mission::{
    ExecutionTarget, MissionBuildError, MissionConfig, MissionEngine, MissionEvent, MissionState,
    OwnshipSample, OwnshipSource, SurveyError, SurveyPlanRecord, SurveyRequest, TruthRole,
    plan_survey,
};
use pilotage_protocol::{ControlAction, ControlIntent, ReferenceFrame};

const CLOCK: ClockDomainId = ClockDomainId::new(7);
const STEP_NANOS: u64 = 50_000_000;
const STEP_S: f64 = 0.05;
const HEIGHT_M: f64 = 15.0;

fn anchor() -> GeodeticPosition {
    GeodeticPosition::new(47.0_f64.to_radians(), 8.0_f64.to_radians(), 500.0)
}

fn plane() -> LocalTangentPlane {
    LocalTangentPlane::new(anchor()).expect("anchor is plausible")
}

/// A 120 m by 60 m rectangle east and north of the anchor, in degrees.
fn rectangle() -> Vec<[f64; 2]> {
    let plane = plane();
    [[0.0, 0.0], [120.0, 0.0], [120.0, 60.0], [0.0, 60.0]]
        .iter()
        .map(|[north, east]| {
            let position = plane.from_ned(&NedOffset::new(*north, *east, 0.0));
            [
                position.latitude_rad.to_degrees(),
                position.longitude_rad.to_degrees(),
            ]
        })
        .collect()
}

fn request() -> SurveyRequest {
    let mut request = SurveyRequest::new(rectangle());
    // A wide footprint keeps the closed loop short: four lines.
    request.across_track_fov_rad = 1.5;
    request
}

fn config(source: OwnshipSource) -> MissionConfig {
    let mut config = MissionConfig::new(String::new(), anchor(), CLOCK);
    config.ownship_source = source;
    config.execution_target = ExecutionTarget::RealVehicle;
    config.cruise_height_m = HEIGHT_M;
    config
}

/// Planar truth integrated from the engine's own body-frame commands.
struct Truth {
    ned: [f64; 3],
    ned_velocity: [f64; 3],
    yaw: f64,
    sequence: u32,
}

impl Truth {
    fn new() -> Self {
        Self {
            ned: [0.0; 3],
            ned_velocity: [0.0; 3],
            yaw: 0.0,
            sequence: 0,
        }
    }

    fn sample(&self, now: MonotonicNanos, role: TruthRole) -> OwnshipSample {
        OwnshipSample {
            ned: self.ned,
            ned_velocity: self.ned_velocity,
            yaw_rad: Some(self.yaw),
            role,
            acquired_at: now,
            sequence: self.sequence,
        }
    }

    fn integrate(&mut self, intent: &ControlIntent) {
        let ControlIntent::Velocity(v) = intent else {
            panic!("mission emits only velocity intents, got {intent:?}");
        };
        assert_eq!(v.frame, ReferenceFrame::BodyFrd);
        let (vx, vy, vz) = (f64::from(v.vx), f64::from(v.vy), f64::from(v.vz));
        let (sin, cos) = self.yaw.sin_cos();
        self.ned_velocity = [vx * cos - vy * sin, vx * sin + vy * cos, vz];
        for axis in 0..3 {
            self.ned[axis] += self.ned_velocity[axis] * STEP_S;
        }
        self.yaw += f64::from(v.yaw_rate) * STEP_S;
    }
}

fn assert_record(record: &SurveyPlanRecord) {
    assert_eq!(record.line_count, 4);
    assert_eq!(record.waypoint_count, 9, "eight line ends and home");
    assert_eq!(record.idents[0], "L01A");
    assert_eq!(record.idents[7], "L04B");
    // The last line is pulled back onto the far side, so the shortest
    // gap bounds the capture radius below the 5 m request.
    assert!(record.capture_radius_m > 1.0 && record.capture_radius_m <= 5.0);
    assert!(
        record
            .mission_identity
            .revision_id
            .starts_with("operational-survey-v1:")
    );
    assert_eq!(
        record
            .mission_identity
            .navigation_data_identity
            .as_ref()
            .map(|identity| identity.cycle.as_str()),
        Some("survey")
    );
    assert_eq!(
        record
            .mission_identity
            .navigation_data_identity
            .as_ref()
            .map(|identity| identity.snapshot_digest),
        Some(record.digest)
    );
}

/// Where each line end sits in the truth frame: north, east.
fn line_ends() -> Vec<[f64; 2]> {
    let plane = plane();
    plan_survey(&request(), &anchor(), HEIGHT_M)
        .expect("plans")
        .plan
        .waypoints
        .iter()
        .map(|waypoint| {
            let offset = plane.to_ned(&waypoint.position);
            [offset.north_m, offset.east_m]
        })
        .collect()
}

/// Runs the closed loop to completion; returns the events and the nearest
/// pass to each line end.
fn run(engine: &mut MissionEngine, role: TruthRole) -> (Vec<MissionEvent>, Vec<f64>, Truth) {
    let ends = line_ends();
    let mut nearest = vec![f64::INFINITY; ends.len()];
    let mut truth = Truth::new();
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let mut events = Vec::new();
    let mut completed = false;
    for _ in 0..40_000 {
        now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
        truth.sequence = truth.sequence.wrapping_add(1);
        engine.on_ownship(&truth.sample(now, role), now);
        let output = engine.tick(now);
        if let Some(action) = output.action {
            assert!(matches!(action.action, ControlAction::Arm));
            engine.on_action_result(action.action_id, true);
        }
        if let Some(intent) = output.intent.as_ref() {
            truth.integrate(intent);
        }
        for (index, end) in ends.iter().enumerate() {
            let distance = (truth.ned[0] - end[0]).hypot(truth.ned[1] - end[1]);
            nearest[index] = nearest[index].min(distance);
        }
        events.extend(output.events.iter().cloned());
        if output.state == MissionState::Complete {
            completed = true;
            break;
        }
    }
    assert!(completed, "the survey completes within the step budget");
    (events, nearest, truth)
}

fn fly(source: OwnshipSource, role: TruthRole) {
    let (mut engine, record) =
        MissionEngine::from_survey(&request(), config(source)).expect("survey builds");
    assert_record(&record);
    assert_eq!(
        engine.mission_document().execution_policy.target,
        ExecutionTarget::RealVehicle
    );
    let (events, nearest, truth) = run(&mut engine, role);
    let advances: Vec<usize> = events
        .iter()
        .filter_map(|event| match event {
            MissionEvent::LegAdvanced { to_index, reason } => {
                assert_eq!(
                    *reason,
                    SequenceReason::Overflown,
                    "fly-over ends capture by overflight"
                );
                Some(*to_index)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        advances,
        (1..9).collect::<Vec<_>>(),
        "every line end is flown in order, then home"
    );
    for (index, distance) in nearest.iter().enumerate() {
        assert!(
            *distance <= record.capture_radius_m + 0.5,
            "line end {index} was passed {distance} m away"
        );
    }
    assert_eq!(engine.counters().guidance_refused, 0);
    assert_eq!(engine.counters().rejected_role, 0);
    let height = -truth.ned[2];
    assert!(
        (height - HEIGHT_M).abs() < 2.0,
        "flown at the survey height, got {height}"
    );
    let from_home = truth.ned[0].hypot(truth.ned[1]);
    assert!(
        from_home <= record.capture_radius_m + 0.5,
        "the mission completes over the launch point, {from_home} m away"
    );
}

#[test]
fn a_survey_is_flown_line_by_line_on_simulation_truth() {
    fly(OwnshipSource::SimulationTruth, TruthRole::SimulationTruth);
}

#[test]
fn a_survey_is_flown_line_by_line_on_the_vehicle_estimate() {
    fly(
        OwnshipSource::vehicle_estimate(),
        TruthRole::OperationalEstimate,
    );
}

/// The first action a fresh engine asks for, once its ownship is valid.
fn first_action(engine: &mut MissionEngine, role: TruthRole) -> Option<ControlAction> {
    let mut truth = Truth::new();
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    for _ in 0..40 {
        now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
        truth.sequence = truth.sequence.wrapping_add(1);
        engine.on_ownship(&truth.sample(now, role), now);
        let output = engine.tick(now);
        if let Some(action) = output.action {
            return Some(action.action);
        }
        if output.intent.is_some() {
            return None;
        }
    }
    None
}

#[test]
fn a_real_vehicle_starts_from_the_hover_and_a_simulator_arms_first() {
    let (mut real, _) =
        MissionEngine::from_survey(&request(), config(OwnshipSource::vehicle_estimate()))
            .expect("survey builds");
    assert!(
        first_action(&mut real, TruthRole::OperationalEstimate).is_none(),
        "a pilot-launched vehicle is never asked to arm"
    );
    assert_eq!(real.mission_document().phases.len(), 2);
    let mut simulated = config(OwnshipSource::SimulationTruth);
    simulated.execution_target = ExecutionTarget::Simulator;
    let (mut simulator, _) =
        MissionEngine::from_survey(&request(), simulated).expect("survey builds");
    assert!(matches!(
        first_action(&mut simulator, TruthRole::SimulationTruth),
        Some(ControlAction::Arm)
    ));
    assert_eq!(simulator.mission_document().phases.len(), 3);
}

#[test]
fn a_survey_that_does_not_plan_refuses_the_engine() {
    let mut request = request();
    request.polygon_deg.truncate(2);
    let error = MissionEngine::from_survey(&request, config(OwnshipSource::SimulationTruth))
        .expect_err("two vertices");
    assert!(matches!(
        error,
        MissionBuildError::Survey(SurveyError::TooFewVertices { count: 2 })
    ));
}
