#![allow(clippy::expect_used, clippy::panic)]

//! Hold and resume: while a human holds the motion scope the engine
//! commands nothing and its sequencing clock pauses; on resume it flies
//! the active leg to completion.

use navigate_contract::{ClockDomainId, GeodeticPosition, MonotonicNanos};
use navigate_geodesy::{LocalTangentPlane, NedOffset};
use pilotage_mission::{
    ExecutionTarget, MissionConfig, MissionEngine, MissionEvent, MissionState, OwnshipSample,
    SurveyRequest, TruthRole,
};
use pilotage_protocol::{ControlAction, ControlIntent, ReferenceFrame};

const CLOCK: ClockDomainId = ClockDomainId::new(7);
const STEP_NANOS: u64 = 50_000_000;
const STEP_S: f64 = 0.05;
const HEIGHT_M: f64 = 15.0;
/// Longer than the 23-hour follow-plan phase deadline, shorter than the
/// 24-hour mission wall deadline.
const LONG_HOLD_NANOS: u64 = 23 * 3_600_000_000_000 + 1_800_000_000_000;

fn anchor() -> GeodeticPosition {
    GeodeticPosition::new(47.0_f64.to_radians(), 8.0_f64.to_radians(), 500.0)
}

fn request() -> SurveyRequest {
    let plane = LocalTangentPlane::new(anchor()).expect("anchor is plausible");
    let polygon = [[0.0, 0.0], [120.0, 0.0], [120.0, 60.0], [0.0, 60.0]]
        .iter()
        .map(|[north, east]| {
            let position = plane.from_ned(&NedOffset::new(*north, *east, 0.0));
            [
                position.latitude_rad.to_degrees(),
                position.longitude_rad.to_degrees(),
            ]
        })
        .collect();
    let mut request = SurveyRequest::new(polygon);
    request.across_track_fov_rad = 1.5;
    request
}

fn engine() -> MissionEngine {
    let mut config = MissionConfig::new(String::new(), anchor(), CLOCK);
    config.execution_target = ExecutionTarget::RealVehicle;
    config.cruise_height_m = HEIGHT_M;
    MissionEngine::from_survey(&request(), config)
        .expect("survey builds")
        .0
}

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

    fn sample(&self, now: MonotonicNanos) -> OwnshipSample {
        OwnshipSample {
            ned: self.ned,
            ned_velocity: self.ned_velocity,
            yaw_rad: Some(self.yaw),
            role: TruthRole::SimulationTruth,
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

/// One closed-loop step; returns the output.
fn step(
    engine: &mut MissionEngine,
    truth: &mut Truth,
    now: &mut MonotonicNanos,
    events: &mut Vec<MissionEvent>,
) -> pilotage_mission::MissionOutput {
    *now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
    truth.sequence = truth.sequence.wrapping_add(1);
    engine.on_ownship(&truth.sample(*now), *now);
    let out = engine.tick(*now);
    if let Some(action) = out.action {
        assert!(matches!(action.action, ControlAction::Arm));
        engine.on_action_result(action.action_id, true);
    }
    if let Some(intent) = out.intent.as_ref() {
        truth.integrate(intent);
    }
    events.extend(out.events.iter().cloned());
    out
}

/// Steps until the mission flies a leg, returning the step count.
fn fly_until_enroute(
    engine: &mut MissionEngine,
    truth: &mut Truth,
    now: &mut MonotonicNanos,
    events: &mut Vec<MissionEvent>,
) {
    for _ in 0..4_000 {
        let output = step(engine, truth, now, events);
        if output.state == MissionState::Enroute && output.intent.is_some() {
            return;
        }
    }
    panic!("the mission never reached enroute guidance");
}

fn count(events: &[MissionEvent], predicate: impl Fn(&MissionEvent) -> bool) -> usize {
    events.iter().filter(|event| predicate(event)).count()
}

#[test]
fn a_held_mission_commands_nothing_and_pauses_its_phase_clock() {
    let mut engine = engine();
    let mut truth = Truth::new();
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let mut events = Vec::new();
    fly_until_enroute(&mut engine, &mut truth, &mut now, &mut events);
    assert!(engine.nav_guidance().is_some());

    engine.hold(now);
    assert!(engine.is_held());
    assert_eq!(engine.state(), MissionState::Held);
    engine.hold(now);
    // A human flies: the truth moves off the line while the engine is held.
    for _ in 0..40 {
        truth.ned[1] += 0.5;
        let output = step(&mut engine, &mut truth, &mut now, &mut events);
        assert_eq!(output.state, MissionState::Held);
        assert!(output.intent.is_none() && output.action.is_none());
        assert!(engine.nav_guidance().is_none(), "no guidance while held");
    }
    assert_eq!(
        count(&events, |e| matches!(e, MissionEvent::MissionHeld)),
        1
    );

    // The hold outlasts the follow-plan phase deadline on the engine
    // clock; the paused sequencing clock keeps the phase alive.
    now = MonotonicNanos::from_nanos(now.as_nanos() + LONG_HOLD_NANOS);
    let held = engine.resume(now).expect("the mission was held");
    assert!(held.as_nanos() >= LONG_HOLD_NANOS);
    assert!(!engine.is_held());
    assert!(
        engine.resume(now).is_none(),
        "resume without a hold is inert"
    );

    let mut completed = false;
    let mut resumed_guidance = false;
    for _ in 0..40_000 {
        let output = step(&mut engine, &mut truth, &mut now, &mut events);
        assert_ne!(
            output.state,
            MissionState::Failed,
            "a paused phase clock does not expire"
        );
        if output.state == MissionState::Enroute && engine.nav_guidance().is_some() {
            resumed_guidance = true;
        }
        if output.state == MissionState::Complete {
            completed = true;
            break;
        }
    }
    assert!(resumed_guidance, "guidance returns after the resume");
    assert!(completed, "the resumed mission completes");
    assert_eq!(
        count(
            &events,
            |e| matches!(e, MissionEvent::MissionResumed { held_ns } if *held_ns >= LONG_HOLD_NANOS)
        ),
        1
    );
    assert_eq!(
        count(&events, |e| matches!(e, MissionEvent::MissionComplete)),
        1
    );
}

#[test]
fn an_unpaused_hold_of_that_length_would_have_expired_the_phase() {
    let mut engine = engine();
    let mut truth = Truth::new();
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let mut events = Vec::new();
    fly_until_enroute(&mut engine, &mut truth, &mut now, &mut events);
    // Same gap, no hold: the follow-plan phase deadline passes.
    now = MonotonicNanos::from_nanos(now.as_nanos() + LONG_HOLD_NANOS);
    let output = step(&mut engine, &mut truth, &mut now, &mut events);
    assert_eq!(output.state, MissionState::Failed);
}

#[test]
fn a_terminal_mission_is_not_held() {
    let mut engine = engine();
    let mut truth = Truth::new();
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let mut events = Vec::new();
    for _ in 0..40_000 {
        if step(&mut engine, &mut truth, &mut now, &mut events).state == MissionState::Complete {
            break;
        }
    }
    assert_eq!(engine.state(), MissionState::Complete);
    engine.hold(now);
    assert!(!engine.is_held());
    assert_eq!(engine.state(), MissionState::Complete);
    assert!(engine.resume(now).is_none());
}
