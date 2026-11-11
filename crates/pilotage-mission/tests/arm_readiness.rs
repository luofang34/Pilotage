//! The arm request waits for a flight controller that says it is ready,
//! and the mission streams intents from the arm request on.

#![allow(clippy::expect_used, clippy::panic)]

use navigate_contract::{ClockDomainId, GeodeticPosition, MonotonicNanos};
use pilotage_mission::fixture::{self, GeoPointDegrees};
use pilotage_mission::{
    MissionConfig, MissionEngine, MissionEvent, MissionOutput, MissionState, OwnshipSample,
    OwnshipSource, StartWithheldReason, TruthRole, VEHICLE_STATUS_MAX_AGE_NS, VehicleStatus,
    decode_snapshot,
};
use pilotage_protocol::{ControlAction, ControlIntent};

const ANCHOR: GeoPointDegrees = GeoPointDegrees {
    lat_deg: 47.0,
    lon_deg: 8.0,
    alt_m: 500.0,
};
const STEP_NS: u64 = 50_000_000;

fn engine(arm_when_vehicle_ready: bool) -> MissionEngine {
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
    config.cruise_height_m = 15.0;
    config.ownship_source = OwnshipSource::SimulationTruth;
    config.arm_when_vehicle_ready = arm_when_vehicle_ready;
    MissionEngine::new(&snapshot, provenance, config)
        .expect("mission builds")
        .0
}

/// A vehicle on the ground at the anchor, ticked every 50 ms.
struct Bench {
    engine: MissionEngine,
    now_ns: u64,
    sequence: u32,
}

impl Bench {
    fn new(arm_when_vehicle_ready: bool) -> Self {
        Self {
            engine: engine(arm_when_vehicle_ready),
            now_ns: 1_000_000_000,
            sequence: 0,
        }
    }

    fn now(&self) -> MonotonicNanos {
        MonotonicNanos::from_nanos(self.now_ns)
    }

    fn tick(&mut self) -> MissionOutput {
        self.now_ns = self.now_ns.wrapping_add(STEP_NS);
        self.sequence = self.sequence.wrapping_add(1);
        let now = self.now();
        let sample = OwnshipSample {
            ned: [0.0; 3],
            ned_velocity: [0.0; 3],
            yaw_rad: Some(0.0),
            role: TruthRole::SimulationTruth,
            acquired_at: now,
            sequence: self.sequence,
        };
        self.engine.on_ownship(&sample, now);
        self.engine.tick(now)
    }

    fn status(&mut self, ready_to_arm: bool) {
        let acquired_at = self.now();
        self.engine
            .on_vehicle_status(VehicleStatus::ready(ready_to_arm, acquired_at));
    }
}

fn withheld(output: &MissionOutput) -> bool {
    output.events.contains(&MissionEvent::StartWithheld {
        reason: StartWithheldReason::VehicleNotReady,
    })
}

#[test]
fn the_arm_request_waits_for_a_ready_vehicle_and_says_why() {
    let mut bench = Bench::new(true);
    bench.status(false);
    let mut withheld_ticks = 0;
    for _ in 0..40 {
        let output = bench.tick();
        assert!(output.action.is_none(), "no arm to a vehicle not ready");
        assert!(
            output.intent.is_none(),
            "nothing streams before an arm request goes out"
        );
        if withheld(&output) {
            withheld_ticks += 1;
        }
    }
    assert_eq!(withheld_ticks, 1, "the hold is announced once");

    bench.status(true);
    let output = bench.tick();
    let action = output
        .action
        .expect("the arm goes out once the vehicle is ready");
    assert_eq!(action.action, ControlAction::Arm);
    assert!(
        output.events.contains(&MissionEvent::ArmRequested {
            action_id: action.action_id
        }),
        "{:?}",
        output.events
    );
}

#[test]
fn a_stale_ready_status_does_not_release_the_arm() {
    let mut bench = Bench::new(true);
    bench.status(true);
    // Older than the freshness bound by the time the arm phase starts.
    bench.now_ns = bench
        .now_ns
        .wrapping_add(VEHICLE_STATUS_MAX_AGE_NS + STEP_NS);
    let held = (0..20)
        .map(|_| bench.tick())
        .all(|output| output.action.is_none());
    assert!(
        held,
        "a ready status from before the bound releases nothing"
    );
    bench.status(true);
    assert!(bench.tick().action.is_some());
}

#[test]
fn without_the_gate_the_arm_goes_out_with_no_status() {
    let mut bench = Bench::new(false);
    let released = (0..40).any(|_| bench.tick().action.is_some());
    assert!(released, "a host that feeds no status still arms");
}

#[test]
fn intents_stream_from_the_arm_request_before_its_confirmation() {
    let mut bench = Bench::new(true);
    bench.status(true);
    let arm = loop {
        let output = bench.tick();
        if let Some(action) = output.action {
            assert_eq!(output.state, MissionState::Arming);
            break action;
        }
    };
    // Before the vehicle confirms the arm, the climb that follows it is
    // already the command.
    for _ in 0..6 {
        let output = bench.tick();
        assert_eq!(output.state, MissionState::Arming);
        let Some(ControlIntent::Velocity(velocity)) = output.intent else {
            panic!("an intent streams while the arm is confirmed: {output:?}");
        };
        assert!(velocity.vz < 0.0, "the climb: {velocity:?}");
        assert_eq!((velocity.vx, velocity.vy), (0.0, 0.0));
    }
    bench.engine.on_action_result(arm.action_id, true);
    let after = bench.tick();
    assert!(after.intent.is_some(), "the climb continues with no gap");
}

#[test]
fn a_long_wait_for_a_ready_vehicle_neither_fails_nor_repeats_the_hold() {
    let mut bench = Bench::new(true);
    let mut withheld_ticks = 0;
    // Thirty seconds of a vehicle that is not ready.
    for _ in 0..600 {
        bench.status(false);
        let output = bench.tick();
        assert!(output.action.is_none());
        assert_ne!(output.state, MissionState::Failed, "{:?}", output.events);
        if withheld(&output) {
            withheld_ticks += 1;
        }
    }
    assert_eq!(withheld_ticks, 1, "the hold is announced once");
    bench.status(true);
    let output = bench.tick();
    assert!(
        output.action.is_some(),
        "the arm goes out once ready: {output:?}"
    );
}
