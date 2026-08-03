//! Deterministic closed-loop mission tests: a kinematic truth model
//! integrates the engine's own commanded intents at 20 Hz, so the loop
//! contains no clock reads, no sleeps, and no randomness.

#![allow(clippy::expect_used, clippy::panic)]

use navigate_contract::{ClockDomainId, GeodeticPosition, MonotonicNanos};
use navigate_fpl::SequenceReason;
use navigate_guidance::GuidanceRefusal;
use pilotage_mission::fixture::{self, GeoPointDegrees};
use pilotage_mission::{
    MissionConfig, MissionEngine, MissionEvent, MissionState, NavGuidance, NavQuality,
    OwnshipSample, TruthRole, decode_snapshot,
};
use pilotage_protocol::{ControlAction, ControlIntent, ReferenceFrame};

const ANCHOR: GeoPointDegrees = GeoPointDegrees {
    lat_deg: 47.0,
    lon_deg: 8.0,
    alt_m: 500.0,
};
const STEP_NANOS: u64 = 50_000_000;
const STEP_S: f64 = 0.05;
const LIMIT_EPS: f64 = 1e-3;

/// Planar truth: NED position and yaw integrated from the engine's own
/// BodyFrd commands (the inverse of the engine's NED-to-body rotation).
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

fn assert_within_limits(intent: &ControlIntent) {
    let ControlIntent::Velocity(v) = intent else {
        panic!("mission emits only velocity intents, got {intent:?}");
    };
    let horizontal = f64::from(v.vx).hypot(f64::from(v.vy));
    assert!(horizontal <= 2.5 + LIMIT_EPS, "horizontal {horizontal}");
    assert!(
        f64::from(v.vz).abs() <= 1.0 + LIMIT_EPS,
        "vertical {}",
        v.vz
    );
    assert!(
        f64::from(v.yaw_rate).abs() <= 0.8 + LIMIT_EPS,
        "yaw rate {}",
        v.yaw_rate
    );
}

fn build_engine() -> (MissionEngine, pilotage_mission::MissionPlanRecord) {
    let blob = fixture::demo_blob(ANCHOR).expect("demo blob encodes");
    let (snapshot, provenance) = decode_snapshot(&blob, true).expect("demo blob decodes");
    let anchor = GeodeticPosition::new(
        ANCHOR.lat_deg.to_radians(),
        ANCHOR.lon_deg.to_radians(),
        ANCHOR.alt_m,
    );
    let config = MissionConfig::new(
        fixture::DEMO_ROUTE.to_owned(),
        anchor,
        ClockDomainId::new(7),
    );
    MissionEngine::new(&snapshot, provenance, config).expect("mission builds")
}

/// One closed-loop step: sample, tick, accept any arm, integrate any
/// intent, collect events.
fn step(
    engine: &mut MissionEngine,
    truth: &mut Truth,
    now: &mut MonotonicNanos,
    arm_requests: &mut u32,
    events: &mut Vec<MissionEvent>,
) {
    *now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
    truth.sequence = truth.sequence.wrapping_add(1);
    engine.on_ownship(&truth.sample(*now), *now);
    let out = engine.tick(*now);
    if let Some(action) = out.action {
        assert!(matches!(action.action, ControlAction::Arm));
        *arm_requests += 1;
        engine.on_action_result(action.action_id, true);
    }
    if let Some(intent) = out.intent {
        assert_within_limits(&intent);
        truth.integrate(&intent);
    }
    events.extend(out.events);
}

fn count<F: Fn(&MissionEvent) -> bool>(events: &[MissionEvent], predicate: F) -> usize {
    events.iter().filter(|event| predicate(event)).count()
}

/// The full flight surfaces each lifecycle event exactly once, in a
/// refusal-free run.
fn assert_flight_events(events: &[MissionEvent]) {
    assert_eq!(
        count(events, |e| matches!(e, MissionEvent::ArmRequested { .. })),
        1
    );
    assert_eq!(
        count(events, |e| matches!(e, MissionEvent::ArmAccepted { .. })),
        1
    );
    assert_eq!(
        count(events, |e| matches!(e, MissionEvent::ClimbStarted)),
        1
    );
    assert_eq!(
        count(events, |e| matches!(e, MissionEvent::EnrouteStarted)),
        1
    );
    // Every demo advance is capture-driven: at the mission cruise speed
    // the fly-by turn radius is on the order of a meter, so the distance
    // of turn anticipation stays far inside the capture radius and no
    // transition is authorized early.
    for to_index in [1usize, 2] {
        assert_eq!(
            count(events, |e| matches!(
                e,
                MissionEvent::LegAdvanced { to_index: t, reason: SequenceReason::Overflown }
                    if *t == to_index
            )),
            1,
            "leg advance to {to_index}"
        );
    }
    assert_eq!(
        count(events, |e| matches!(e, MissionEvent::MissionComplete)),
        1
    );
    assert_eq!(
        count(events, |e| matches!(
            e,
            MissionEvent::GuidanceRefused { .. }
        )),
        0
    );
}

#[test]
fn mission_arms_once_climbs_flies_the_route_and_completes() {
    let (mut engine, record) = build_engine();
    assert_eq!(record.expanded_idents, vec!["DEMOA", "DEMOB", "DEMOC"]);
    assert_eq!(record.waypoint_count, 3);
    assert!(record.provenance.fixture);

    let mut truth = Truth::new();
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let mut arm_requests = 0;
    let mut events = Vec::new();
    let mut completed = false;
    for _ in 0..20_000 {
        step(
            &mut engine,
            &mut truth,
            &mut now,
            &mut arm_requests,
            &mut events,
        );
        if engine.state() == MissionState::Complete {
            completed = true;
            break;
        }
    }
    assert!(completed, "mission completes within the step budget");
    assert_eq!(
        arm_requests, 1,
        "arm goes out once, never re-sent after acceptance"
    );
    assert_flight_events(&events);
    assert_eq!(engine.counters().guidance_refused, 0);
    assert_eq!(engine.counters().fusion_rejected, 0);

    let height_m = -truth.ned[2];
    assert!(height_m > 10.0, "still at cruise height, got {height_m}");
    let (dn, de) = (truth.ned[0] - (-127.0), truth.ned[1] - 127.0);
    let to_democ = dn.hypot(de);
    assert!(
        to_democ <= 110.0,
        "completed inside DEMOC capture, {to_democ} m out"
    );

    // Completion holds: zero velocity every tick, the event never repeats.
    for _ in 0..3 {
        let mut post_events = Vec::new();
        step(
            &mut engine,
            &mut truth,
            &mut now,
            &mut arm_requests,
            &mut post_events,
        );
        assert_eq!(arm_requests, 1);
        assert!(post_events.is_empty(), "no events after completion");
        assert!(truth.ned_velocity.iter().all(|v| v.abs() < 1e-9));
    }
}

#[test]
fn guidance_is_absent_until_a_leg_is_being_flown() {
    let (mut engine, _) = build_engine();
    assert!(
        engine.nav_guidance().is_none(),
        "no solution yet, so nothing to display"
    );

    // Fly up to the arm request and leave it outstanding: a solution now
    // exists, but the executor is not flying a leg yet.
    let mut truth = Truth::new();
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let mut requested = false;
    for _ in 0..50 {
        now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
        truth.sequence = truth.sequence.wrapping_add(1);
        engine.on_ownship(&truth.sample(now), now);
        if engine.tick(now).action.is_some() {
            requested = true;
            break;
        }
    }
    assert!(requested, "the arm request goes out within the step budget");
    assert_eq!(engine.state(), MissionState::Arming);
    assert!(
        engine.nav_guidance().is_none(),
        "arming is not flying a leg: no guidance display"
    );
}

#[test]
fn guidance_tracks_the_active_leg_and_leaves_with_the_plan() {
    let (mut engine, _) = build_engine();
    let mut truth = Truth::new();
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let mut arm_requests = 0;
    let mut events = Vec::new();
    let mut climb: Option<NavGuidance> = None;
    let mut sequenced: Option<NavGuidance> = None;

    for _ in 0..20_000 {
        step(
            &mut engine,
            &mut truth,
            &mut now,
            &mut arm_requests,
            &mut events,
        );
        if engine.state() == MissionState::Complete {
            break;
        }
        let Some(guidance) = engine.nav_guidance() else {
            continue;
        };
        if engine.state() == MissionState::Climb && climb.is_none() {
            climb = Some(guidance);
        } else if guidance.leg_index > 0 && sequenced.is_none() {
            sequenced = Some(guidance);
        }
    }
    assert_eq!(engine.state(), MissionState::Complete);

    // The climb flies direct-to: guidance is published with the live
    // bearing to the first fix and a real distance, but no lateral course
    // is being tracked, so there is nothing to deviate from.
    let climb = climb.expect("the climb phase publishes guidance");
    assert_eq!(climb.to_ident, "DEMOA");
    assert_eq!(climb.from_ident, None);
    assert_eq!(climb.lateral_deviation_m, None);
    assert_eq!(climb.leg_index, 0);
    assert_eq!(climb.waypoint_count, 3);
    assert!((0.0..std::f64::consts::TAU).contains(&climb.course_rad));
    assert!(
        climb.distance_to_waypoint_m > 0.0 && climb.distance_to_waypoint_m.is_finite(),
        "the climb still reports a real distance to run, got {}",
        climb.distance_to_waypoint_m
    );

    // Past the first capture the leg has an origin fix, so cross-track
    // deviation exists and the course is the leg's track.
    let sequenced = sequenced.expect("a sequenced leg publishes guidance");
    assert_eq!(sequenced.from_ident.as_deref(), Some("DEMOA"));
    assert_eq!(sequenced.to_ident, "DEMOB");
    assert_eq!(sequenced.leg_index, 1);
    assert_eq!(sequenced.quality, NavQuality::Good);
    assert!(sequenced.lateral_deviation_m.is_some_and(f64::is_finite));
    assert!(
        sequenced.vertical_deviation_m.is_some_and(f64::is_finite),
        "every demo waypoint carries an altitude constraint"
    );

    assert!(
        engine.nav_guidance().is_none(),
        "a completed plan removes the guidance display rather than freezing it"
    );
}

#[test]
fn non_truth_roles_are_refused_and_never_drive_the_mission() {
    let (mut engine, _) = build_engine();
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let truth = Truth::new();
    for index in 0..50u32 {
        now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
        let mut sample = truth.sample(now);
        sample.sequence = index;
        sample.role = if index % 2 == 0 {
            TruthRole::FcState
        } else {
            TruthRole::OperationalEstimate
        };
        engine.on_ownship(&sample, now);
        let out = engine.tick(now);
        assert!(out.intent.is_none());
        assert!(out.action.is_none());
        assert_eq!(
            out.state,
            MissionState::AwaitSolution,
            "no laundered solution"
        );
    }
    assert_eq!(engine.counters().rejected_role, 50);
    assert_eq!(engine.counters().fusion_rejected, 0);
}

#[test]
fn observation_silence_refuses_guidance_with_one_surfaced_event() {
    let (mut engine, _) = build_engine();
    let mut truth = Truth::new();
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let mut arm_requests = 0;
    let mut events = Vec::new();
    for _ in 0..2_000 {
        step(
            &mut engine,
            &mut truth,
            &mut now,
            &mut arm_requests,
            &mut events,
        );
        if engine.state() == MissionState::Enroute {
            break;
        }
    }
    assert_eq!(engine.state(), MissionState::Enroute, "reached enroute");

    // Six seconds of observation silence: solution quality decays to
    // Unusable and guidance must refuse rather than guess.
    now = MonotonicNanos::from_nanos(now.as_nanos() + 6_000_000_000);
    let out = engine.tick(now);
    assert!(out.intent.is_none(), "no intent on a refused tick");
    let refusals: Vec<&MissionEvent> = out
        .events
        .iter()
        .filter(|e| matches!(e, MissionEvent::GuidanceRefused { .. }))
        .collect();
    assert_eq!(refusals.len(), 1, "exactly one surfaced refusal");
    assert!(
        matches!(
            refusals[0],
            MissionEvent::GuidanceRefused {
                reason: GuidanceRefusal::IntegrityBelowFloor { .. }
            }
        ),
        "refusal names the integrity floor, got {:?}",
        refusals[0]
    );
    let refused = engine.counters().guidance_refused;
    assert!(refused >= 1);

    // The same refusal kind repeats: counted, not re-surfaced.
    now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
    let out = engine.tick(now);
    assert!(out.intent.is_none());
    assert_eq!(
        count(&out.events, |e| matches!(
            e,
            MissionEvent::GuidanceRefused { .. }
        )),
        0
    );
    assert!(engine.counters().guidance_refused > refused);
}

/// A sample stream that never carries a heading feeds fusion but cannot
/// be rotated into the body frame: no intent, a counted refusal, and
/// flight resumes the moment a heading arrives.
#[test]
fn a_missing_heading_withholds_intents_and_is_counted() {
    // Zero cruise height goes straight to Enroute, where intents need
    // the NED→body rotation (the climb command is body-vertical only).
    let blob = fixture::demo_blob(ANCHOR).expect("demo blob encodes");
    let (snapshot, provenance) = decode_snapshot(&blob, true).expect("demo blob decodes");
    let anchor = GeodeticPosition::new(
        ANCHOR.lat_deg.to_radians(),
        ANCHOR.lon_deg.to_radians(),
        ANCHOR.alt_m,
    );
    let mut config = MissionConfig::new(
        fixture::DEMO_ROUTE.to_owned(),
        anchor,
        ClockDomainId::new(7),
    );
    config.cruise_height_m = 0.0;
    let (mut engine, _record) =
        MissionEngine::new(&snapshot, provenance, config).expect("mission builds");
    let mut truth = Truth::new();
    let mut now = MonotonicNanos::from_nanos(0);
    let mut armed = false;
    let mut saw_missing_yaw_intentless_tick = false;
    for _ in 0..40 {
        now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
        truth.sequence = truth.sequence.wrapping_add(1);
        let mut sample = truth.sample(now);
        sample.yaw_rad = None;
        engine.on_ownship(&sample, now);
        let out = engine.tick(now);
        if let Some(action) = out.action {
            engine.on_action_result(action.action_id, true);
            armed = true;
        }
        if armed && matches!(out.state, MissionState::Enroute) && out.intent.is_none() {
            saw_missing_yaw_intentless_tick = true;
        }
        assert!(
            out.intent.is_none(),
            "an unheaded stream must never produce a body-frame intent"
        );
    }
    assert!(saw_missing_yaw_intentless_tick, "enroute was reached");
    assert!(engine.counters().missing_yaw > 0, "the refusal is counted");

    // One headed sample restores flight on the very next tick.
    now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
    truth.sequence = truth.sequence.wrapping_add(1);
    engine.on_ownship(&truth.sample(now), now);
    let out = engine.tick(now);
    assert!(
        out.intent.is_some(),
        "a known heading resumes intent emission"
    );
}
