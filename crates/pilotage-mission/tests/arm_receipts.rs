//! Correlated arm receipt and resend tests.

#![allow(clippy::expect_used, clippy::panic)]

use navigate_contract::{ClockDomainId, GeodeticPosition, MonotonicNanos};
use pilotage_mission::fixture::{self, GeoPointDegrees};
use pilotage_mission::{
    MissionConfig, MissionEngine, MissionEvent, MissionState, OwnshipSample, TruthRole,
    decode_snapshot,
};

const ANCHOR: GeoPointDegrees = GeoPointDegrees {
    lat_deg: 47.0,
    lon_deg: 8.0,
    alt_m: 500.0,
};
const STEP_NS: u64 = 50_000_000;

fn engine_with_cruise_height(cruise_height_m: f64) -> MissionEngine {
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
    config.cruise_height_m = cruise_height_m;
    MissionEngine::new(&snapshot, provenance, config)
        .expect("mission builds")
        .0
}

fn engine() -> MissionEngine {
    engine_with_cruise_height(15.0)
}

fn sample(now: MonotonicNanos, sequence: u32) -> OwnshipSample {
    OwnshipSample {
        ned: [0.0; 3],
        ned_velocity: [0.0; 3],
        yaw_rad: Some(0.0),
        role: TruthRole::SimulationTruth,
        acquired_at: now,
        sequence,
    }
}

fn tick_with_sample(
    engine: &mut MissionEngine,
    now_ns: &mut u64,
    sequence: &mut u32,
) -> pilotage_mission::MissionOutput {
    *now_ns = now_ns.wrapping_add(STEP_NS);
    *sequence = sequence.wrapping_add(1);
    let now = MonotonicNanos::from_nanos(*now_ns);
    engine.on_ownship(&sample(now, *sequence), now);
    engine.tick(now)
}

#[test]
fn a_rejected_arm_receipt_resends_with_a_fresh_correlated_id() {
    let mut engine = engine();
    let mut now_ns = 1_000_000_000;
    let mut sequence = 0;
    let first = loop {
        let output = tick_with_sample(&mut engine, &mut now_ns, &mut sequence);
        if let Some(action) = output.action {
            break action;
        }
    };
    assert_ne!(first.action_id, 0);

    engine.on_action_result(first.action_id, false);
    let retry = tick_with_sample(&mut engine, &mut now_ns, &mut sequence);
    let second = retry.action.expect("the rejected action is retried");
    assert_ne!(second.action_id, first.action_id);
    assert!(retry.events.iter().any(|event| matches!(
        event,
        MissionEvent::ArmRejected { action_id } if *action_id == first.action_id
    )));
    assert!(retry.events.iter().any(|event| matches!(
        event,
        MissionEvent::ArmRequested { action_id } if *action_id == second.action_id
    )));

    engine.on_action_result(first.action_id, true);
    let waiting = tick_with_sample(&mut engine, &mut now_ns, &mut sequence);
    assert_eq!(waiting.state, MissionState::Arming);
    assert!(waiting.action.is_none(), "the stale receipt is inert");

    engine.on_action_result(second.action_id, true);
    let accepted = tick_with_sample(&mut engine, &mut now_ns, &mut sequence);
    assert_eq!(accepted.state, MissionState::Climb);
    assert!(accepted.events.iter().any(|event| matches!(
        event,
        MissionEvent::ArmAccepted { action_id } if *action_id == second.action_id
    )));
    assert!(
        accepted
            .events
            .iter()
            .any(|event| matches!(event, MissionEvent::ClimbStarted))
    );
    assert_eq!(engine.counters().arm_rejected, 1);
}

#[test]
fn zero_height_starts_enroute_guidance_on_the_arm_receipt_tick() {
    let mut engine = engine_with_cruise_height(0.0);
    let mut now_ns = 1_000_000_000;
    let mut sequence = 0;
    let arm = loop {
        let output = tick_with_sample(&mut engine, &mut now_ns, &mut sequence);
        if let Some(action) = output.action {
            break action;
        }
    };

    engine.on_action_result(arm.action_id, true);
    let output = tick_with_sample(&mut engine, &mut now_ns, &mut sequence);

    assert_eq!(output.state, MissionState::Enroute);
    assert!(
        output.intent.is_some(),
        "enroute guidance has no empty frame"
    );
    assert!(output.events.iter().any(|event| matches!(
        event,
        MissionEvent::ArmAccepted { action_id } if *action_id == arm.action_id
    )));
    assert!(
        output
            .events
            .iter()
            .any(|event| matches!(event, MissionEvent::EnrouteStarted))
    );
    assert!(
        !output
            .events
            .iter()
            .any(|event| matches!(event, MissionEvent::ClimbStarted))
    );
}
