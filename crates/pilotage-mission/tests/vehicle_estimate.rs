#![cfg(test)]

//! The vehicle-estimate ownship source: the engine flies a physical
//! vehicle on its own exported estimate, declared as such, without
//! fusion, and refuses every other role.

use navigate_contract::{ClockDomainId, DurationNanos, GeodeticPosition, MonotonicNanos};
use pilotage_mission::fixture::{self, GeoPointDegrees};
use pilotage_mission::{
    ExecutionTarget, MissionConfig, MissionEngine, MissionEvent, MissionState, NavQuality,
    OwnshipAltitude, OwnshipSample, OwnshipSource, TruthRole, decode_snapshot,
};
use pilotage_protocol::{ControlAction, ControlIntent, ReferenceFrame};

const ANCHOR: GeoPointDegrees = GeoPointDegrees {
    lat_deg: 47.0,
    lon_deg: 8.0,
    alt_m: 500.0,
};
const CLOCK: ClockDomainId = ClockDomainId::new(7);
const STEP_NANOS: u64 = 50_000_000;
const STEP_S: f64 = 0.05;

/// A flight controller's exported estimate: NED position and yaw
/// integrated from the engine's own body-frame commands.
struct Estimate {
    ned: [f64; 3],
    ned_velocity: [f64; 3],
    yaw: f64,
    sequence: u32,
}

impl Estimate {
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

fn anchor() -> GeodeticPosition {
    GeodeticPosition::new(
        ANCHOR.lat_deg.to_radians(),
        ANCHOR.lon_deg.to_radians(),
        ANCHOR.alt_m,
    )
}

fn build_engine(source: OwnshipSource) -> MissionEngine {
    let blob = fixture::demo_blob(ANCHOR).expect("demo blob encodes");
    let (snapshot, provenance) = decode_snapshot(&blob, true).expect("demo blob decodes");
    let mut config = MissionConfig::new(fixture::DEMO_ROUTE.to_owned(), anchor(), CLOCK);
    config.ownship_source = source;
    config.execution_target = ExecutionTarget::RealVehicle;
    MissionEngine::new(&snapshot, provenance, config)
        .expect("mission builds")
        .0
}

/// One closed-loop step under the estimate role.
fn step(
    engine: &mut MissionEngine,
    estimate: &mut Estimate,
    now: &mut MonotonicNanos,
    events: &mut Vec<MissionEvent>,
) -> pilotage_mission::MissionOutput {
    *now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
    estimate.sequence = estimate.sequence.wrapping_add(1);
    engine.on_ownship(&estimate.sample(*now, TruthRole::OperationalEstimate), *now);
    let out = engine.tick(*now);
    if let Some(action) = out.action {
        assert!(matches!(action.action, ControlAction::Arm));
        engine.on_action_result(action.action_id, true);
    }
    if let Some(intent) = out.intent.as_ref() {
        estimate.integrate(intent);
    }
    events.extend(out.events.iter().cloned());
    out
}

#[test]
fn the_document_is_admitted_for_a_real_vehicle() {
    let engine = build_engine(OwnshipSource::vehicle_estimate());
    assert_eq!(
        engine.mission_document().execution_policy.target,
        ExecutionTarget::RealVehicle
    );
}

#[test]
fn only_the_estimate_role_is_admitted_and_truth_is_a_counted_refusal() {
    let mut engine = build_engine(OwnshipSource::vehicle_estimate());
    let estimate = Estimate::new();
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    for role in [TruthRole::SimulationTruth, TruthRole::FcState] {
        for _ in 0..10 {
            now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
            engine.on_ownship(&estimate.sample(now, role), now);
            let output = engine.tick(now);
            assert_eq!(output.state, MissionState::AwaitSolution);
            assert!(output.intent.is_none() && output.action.is_none());
        }
    }
    assert_eq!(engine.counters().rejected_role, 20);
    assert_eq!(engine.counters().fusion_rejected, 0);
    assert!(engine.nav_guidance().is_none());

    now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
    engine.on_ownship(&estimate.sample(now, TruthRole::OperationalEstimate), now);
    let output = engine.tick(now);
    assert_ne!(output.state, MissionState::AwaitSolution);
    assert_eq!(engine.counters().rejected_role, 20);
}

#[test]
fn the_estimate_flies_the_route_to_completion_as_a_degraded_solution() {
    let mut engine = build_engine(OwnshipSource::vehicle_estimate());
    let mut estimate = Estimate::new();
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let mut events = Vec::new();
    let mut completed = false;
    let mut guidance_seen = false;
    for _ in 0..20_000 {
        let output = step(&mut engine, &mut estimate, &mut now, &mut events);
        if let Some(guidance) = engine.nav_guidance() {
            assert_eq!(guidance.quality, NavQuality::Degraded);
            guidance_seen = true;
        }
        if output.state == MissionState::Complete {
            completed = true;
            break;
        }
    }
    assert!(completed, "mission completes within the step budget");
    assert!(guidance_seen, "guidance was displayed while flying a leg");
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, MissionEvent::MissionComplete))
            .count(),
        1
    );
    assert_eq!(engine.counters().guidance_refused, 0);
    assert_eq!(engine.counters().rejected_role, 0);
    assert_eq!(engine.counters().fusion_rejected, 0);
    assert_eq!(engine.counters().estimate_stale, 0);
    let height_m = -estimate.ned[2];
    assert!(height_m > 10.0, "still at cruise height, got {height_m}");
}

#[test]
fn a_stale_estimate_publishes_no_solution_and_commands_nothing() {
    let max_age = DurationNanos::from_millis(400);
    let mut engine = build_engine(OwnshipSource::VehicleEstimate {
        sigma_m: 3.0,
        velocity_sigma_mps: 0.5,
        max_age,
    });
    let mut estimate = Estimate::new();
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let mut events = Vec::new();
    let mut flying = false;
    for _ in 0..2_000 {
        let output = step(&mut engine, &mut estimate, &mut now, &mut events);
        if output.state == MissionState::Enroute && output.intent.is_some() {
            flying = true;
            break;
        }
    }
    assert!(flying, "the mission reaches enroute guidance");
    assert!(engine.nav_guidance().is_some());

    // The estimate stops. Within the age budget the last sample still
    // flies; past it nothing is commanded and the display clears.
    let fresh = MonotonicNanos::from_nanos(now.as_nanos() + max_age.as_nanos() / 2);
    let output = engine.tick(fresh);
    assert!(output.intent.is_some(), "a sample inside the budget flies");
    let stale = MonotonicNanos::from_nanos(now.as_nanos() + max_age.as_nanos() + 1);
    let stale_before = engine.counters().estimate_stale;
    let output = engine.tick(stale);
    assert!(output.intent.is_none(), "a stale sample commands nothing");
    assert_eq!(output.state, MissionState::Enroute);
    assert_eq!(engine.counters().estimate_stale, stale_before + 1);
    assert!(
        engine.nav_guidance().is_none(),
        "no solution, no guidance display"
    );
}

#[test]
fn ned_of_measures_from_the_anchor_and_reads_heights_above_it() {
    let engine = build_engine(OwnshipSource::vehicle_estimate());
    let at_anchor = engine.ned_of(
        anchor().latitude_rad,
        anchor().longitude_rad,
        OwnshipAltitude::Ellipsoid(ANCHOR.alt_m),
    );
    for component in at_anchor {
        assert!(
            component.abs() < 1e-6,
            "anchor is the origin, got {at_anchor:?}"
        );
    }
    let above = engine.ned_of(
        anchor().latitude_rad,
        anchor().longitude_rad,
        OwnshipAltitude::AboveAnchor(110.0),
    );
    assert!(
        (above[2] + 110.0).abs() < 1e-6,
        "110 m above is -110 m down, got {above:?}"
    );
    let north = engine.ned_of(
        (ANCHOR.lat_deg + 0.001).to_radians(),
        anchor().longitude_rad,
        OwnshipAltitude::Ellipsoid(ANCHOR.alt_m),
    );
    assert!(
        (north[0] - 111.2).abs() < 0.5,
        "0.001 degrees north, got {north:?}"
    );
    assert!(north[1].abs() < 0.01);
}

#[test]
fn the_truth_source_keeps_refusing_the_estimate_role() {
    let mut engine = build_engine(OwnshipSource::SimulationTruth);
    let estimate = Estimate::new();
    let now = MonotonicNanos::from_nanos(1_000_000_000);
    engine.on_ownship(&estimate.sample(now, TruthRole::OperationalEstimate), now);
    assert_eq!(engine.counters().rejected_role, 1);
    assert_eq!(engine.tick(now).state, MissionState::AwaitSolution);
}

#[test]
fn a_missing_heading_does_not_reuse_the_previous_body_transform() {
    let mut engine = build_engine(OwnshipSource::vehicle_estimate());
    let mut estimate = Estimate::new();
    let mut now = MonotonicNanos::from_nanos(1_000_000_000);
    let mut events = Vec::new();
    let mut moving = false;
    for _ in 0..2_000 {
        let output = step(&mut engine, &mut estimate, &mut now, &mut events);
        if output.state == MissionState::Enroute && output.intent.is_some() {
            moving = true;
            break;
        }
    }
    assert!(moving);
    now = MonotonicNanos::from_nanos(now.as_nanos() + STEP_NANOS);
    let mut sample = estimate.sample(now, TruthRole::OperationalEstimate);
    sample.yaw_rad = None;
    engine.on_ownship(&sample, now);
    let output = engine.tick(now);
    assert!(output.intent.is_none());
    assert!(engine.counters().missing_yaw > 0);
}
