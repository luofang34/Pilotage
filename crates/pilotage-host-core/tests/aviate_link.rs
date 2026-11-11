#![cfg(test)]

//! The core against the traffic of Aviate's C172 board: the flight
//! controller's datagrams of a recorded run (`fixtures/`, 15 s, the
//! format `<ns> from_fc|to_fc <hex>`) are replayed into the core at their
//! times, with the plan loaded at 2 s as in the recorded run. The core
//! must arm the aircraft, keep the lease holder's frames flowing, and give
//! the same bytes on every run.

use std::time::Duration;

use navigate_contract::{GeodeticPosition, MonotonicNanos};
use pilotage_adapter_api::SourceIncarnation;
use pilotage_adapter_aviate::{AviateAirframe, EmbeddedIncarnations};
use pilotage_control_feel::{FixedWingEnvelope, ValidatedFixedWingEnvelope};
use pilotage_host_core::{HostCore, HostCoreConfig, HostEvent, MISSION_CLOCK};
use pilotage_mavlink::link::LinkConfig;
use pilotage_mission::{MissionConfig, MissionEvent, OwnshipSource, PlannedWaypoint, WaypointPlan};
use pilotage_protocol::VehicleClass;
use web_time::Instant;

const CAPTURE: &str = include_str!("fixtures/aviate-c172-link-15s.txt");
/// The closed-loop power-up run's traffic with the board kept in BOOT for
/// 8 s (its heartbeats rewritten, the recorded arm's answer removed).
const BOOT_8S: &str = include_str!("fixtures/aviate-c172-boot-8s.txt");
const LAT_DEG: f64 = 41.7;
const LON_DEG: f64 = -1.07;
const ALT_M: f64 = 2050.0;

fn config() -> HostCoreConfig {
    let anchor = GeodeticPosition::new(LAT_DEG.to_radians(), LON_DEG.to_radians(), ALT_M);
    let mut mission = MissionConfig::new(String::new(), anchor, MISSION_CLOCK);
    mission.ownship_source = OwnshipSource::vehicle_estimate();
    mission.vehicle_class = VehicleClass::FixedWing;
    mission.cruise_height_m = 0.0;
    mission.cruise_mps = 55.0;
    mission.limits.max_horizontal_mps = 65.0;
    // The default link configuration: the adapter supplies Aviate's
    // inter-group skew budget.
    HostCoreConfig {
        link: LinkConfig::default(),
        airframe: AviateAirframe::FixedWing(
            ValidatedFixedWingEnvelope::new(FixedWingEnvelope::C172).unwrap(),
        ),
        fc_origin_height_m: Some(ALT_M),
        incarnations: EmbeddedIncarnations {
            estimate: SourceIncarnation::new([1; 16]),
            fc_state: SourceIncarnation::new([2; 16]),
        },
        mission,
        magnetic_epoch_unix_s: 1_790_000_000,
    }
}

fn plan() -> WaypointPlan {
    WaypointPlan::new(vec![
        PlannedWaypoint::new(LAT_DEG, LON_DEG + 0.03, ALT_M),
        PlannedWaypoint::new(LAT_DEG + 0.02, LON_DEG + 0.06, ALT_M),
    ])
}

/// The FC's datagrams of `capture`, with their times.
fn from_fc(capture: &str) -> Vec<(u64, Vec<u8>)> {
    capture
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let at = parts.next()?.parse::<u64>().ok()?;
            (parts.next()? == "from_fc").then_some(())?;
            Some((at, hex(parts.next()?)?))
        })
        .collect()
}

fn hex(text: &str) -> Option<Vec<u8>> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok())
        .collect()
}

/// What one replay gives: the frames the core sent and its events, each
/// with its time, and the core.
struct Replay {
    sent: Vec<(u64, Vec<u8>)>,
    events: Vec<(u64, HostEvent)>,
    core: HostCore,
}

fn replay(load_at_ns: u64) -> Replay {
    replay_of(CAPTURE, load_at_ns)
}

fn replay_of(capture: &str, load_at_ns: u64) -> Replay {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init()
        .ok();
    let mut core = HostCore::new(config(), Instant::now()).unwrap();
    let mut sent = Vec::new();
    let mut events = Vec::new();
    let mut wake = MonotonicNanos::from_nanos(0);
    let mut loaded = false;
    let mut run_to = |core: &mut HostCore, until: u64, wake: &mut MonotonicNanos| {
        while wake.as_nanos() <= until {
            let at = wake.as_nanos();
            *wake = core.tick(*wake).unwrap().at;
            sent.extend(core.drain_uplink().into_iter().map(|frame| (at, frame)));
            events.extend(core.drain_events().into_iter().map(|event| (at, event)));
        }
    };
    for (at, datagram) in from_fc(capture) {
        if !loaded && at >= load_at_ns {
            run_to(&mut core, load_at_ns, &mut wake);
            core.load_mission(plan()).unwrap();
            loaded = true;
        }
        run_to(&mut core, at, &mut wake);
        let now = MonotonicNanos::from_nanos(at);
        core.on_downlink(&datagram, now);
        core.on_uplink_reply(&datagram, now);
    }
    run_to(&mut core, 15_000_000_000, &mut wake);
    Replay { sent, events, core }
}

#[test]
fn the_core_arms_the_c172_and_keeps_framing() {
    let Replay { sent, events, core } = replay(2_000_000_000);
    for (at, event) in &events {
        tracing::info!(
            at_s = Duration::from_nanos(*at).as_secs_f64(),
            ?event,
            "core event"
        );
    }
    let armed = events
        .iter()
        .find(|(_, event)| matches!(event, HostEvent::ArmConfirmed))
        .map(|(at, _)| *at)
        .expect("the core armed the aircraft");
    let requested = events
        .iter()
        .find(|(_, event)| matches!(event, HostEvent::Mission(MissionEvent::ArmRequested { .. })))
        .map(|(at, _)| *at)
        .expect("the core asked for the arm");
    assert!(
        !events.iter().any(|(_, event)| matches!(
            event,
            HostEvent::Mission(MissionEvent::MissionFailed { .. })
        )),
        "the mission did not fail"
    );
    // From the arm request on, through the wait for the arm's receipt,
    // the holder frames the FC at the mission cadence.
    let after: Vec<u64> = sent
        .iter()
        .filter(|(at, _)| *at >= requested)
        .map(|(at, _)| *at)
        .collect();
    let longest_gap = after
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .max()
        .unwrap_or(u64::MAX);
    tracing::info!(
        armed_s = Duration::from_nanos(armed).as_secs_f64(),
        frames_after = after.len(),
        longest_gap_ms = longest_gap / 1_000_000,
        status = ?core.status(),
        "replay done"
    );
    assert!(after.len() > 100, "{} frames after the arm", after.len());
    assert_eq!(
        core.status().frames_rejected,
        0,
        "every frame met a measurement"
    );
    assert!(
        core.status().estimate.is_some(),
        "the core reports the estimate it flies"
    );
    assert!(
        longest_gap <= 200_000_000,
        "a {longest_gap} ns gap in the frames"
    );
}

#[test]
fn the_same_traffic_gives_the_same_bytes() {
    let first = replay(2_000_000_000);
    let second = replay(2_000_000_000);
    assert_eq!(first.sent, second.sent);
    assert_eq!(first.core.status(), second.core.status());
    assert_ne!(
        first.core.status().frame_digest,
        [0; 32],
        "frames were sent"
    );
}

#[test]
fn the_arm_waits_until_the_c172_reports_itself_ready() {
    // Loaded at 0.1 s, while the board still boots: its heartbeat says
    // STANDBY from 1.008 s on.
    let Replay { events, .. } = replay(100_000_000);
    let withheld = events.iter().any(|(_, event)| {
        matches!(
            event,
            HostEvent::Mission(MissionEvent::StartWithheld { .. })
        )
    });
    assert!(withheld, "the engine said it waits for the vehicle");
    let requested = events
        .iter()
        .find(|(_, event)| matches!(event, HostEvent::Mission(MissionEvent::ArmRequested { .. })))
        .map(|(at, _)| *at)
        .expect("the core asked for the arm once the board was ready");
    assert!(requested >= 1_008_000_000, "asked at {requested} ns");
    assert!(
        !events.iter().any(|(_, event)| matches!(
            event,
            HostEvent::ArmRefused | HostEvent::Mission(MissionEvent::MissionFailed { .. })
        )),
        "no arm is refused"
    );
}

#[test]
fn a_board_that_boots_for_eight_seconds_keeps_the_mission_alive_until_it_arms() {
    // Loaded at power-up; the board says BOOT until 8 s, then STANDBY.
    let Replay { sent, events, core } = replay_of(BOOT_8S, 0);
    let requested = events
        .iter()
        .find(|(_, event)| matches!(event, HostEvent::Mission(MissionEvent::ArmRequested { .. })))
        .map(|(at, _)| *at)
        .expect("the core asked for the arm once the board was ready");
    assert!(requested >= 8_000_000_000, "asked at {requested} ns");
    // The holder frames through the whole wait, so the session never
    // finds it silent and fences the mission.
    let times: Vec<u64> = sent.iter().map(|(at, _)| *at).collect();
    let longest_gap = times
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .max()
        .unwrap_or(u64::MAX);
    assert!(
        longest_gap <= 200_000_000,
        "a {longest_gap} ns gap in the frames"
    );
    // A fenced principal stops framing; this one frames to the end. (The
    // adapter refuses the first frames for want of a measurement before
    // the board streams one; that is not a fence.)
    let last = times.last().copied().unwrap_or_default();
    assert!(last >= 14_900_000_000, "the frames stopped at {last} ns");
    assert!(core.status().frames_sent > 250, "{:?}", core.status());
    assert!(
        !events.iter().any(|(_, event)| matches!(
            event,
            HostEvent::Mission(MissionEvent::MissionFailed { .. })
        )),
        "the mission did not fail"
    );
}
