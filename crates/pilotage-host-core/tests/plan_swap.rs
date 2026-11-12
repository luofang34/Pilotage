#![cfg(test)]

//! A plan loaded while another one flies: the core against the traffic of
//! Aviate's C172 board (the `aviate_link` capture), with the first plan
//! loaded at 2 s and a second one at 10 s. The flight controller starts its
//! command-loss descent after 500 ms without a setpoint, so the setpoints
//! must not stop while the new plan takes over the motion scope. A plan
//! that does not build leaves the flown plan flying.

use navigate_contract::{GeodeticPosition, MonotonicNanos};
use pilotage_adapter_api::SourceIncarnation;
use pilotage_adapter_aviate::{AviateAirframe, EmbeddedIncarnations};
use pilotage_control_feel::{FixedWingEnvelope, ValidatedFixedWingEnvelope};
use pilotage_host_core::{HostCore, HostCoreConfig, HostEvent, MISSION_CLOCK};
use pilotage_mavlink::link::LinkConfig;
use pilotage_mission::{
    MissionConfig, MissionEvent, MissionState, OwnshipSource, PlannedWaypoint, WaypointPlan,
};
use pilotage_protocol::VehicleClass;
use web_time::Instant;

const CAPTURE: &str = include_str!("fixtures/aviate-c172-link-15s.txt");
const LAT_DEG: f64 = 41.7;
const LON_DEG: f64 = -1.07;
const ALT_M: f64 = 2050.0;
const FIRST_AT_NS: u64 = 2_000_000_000;
const SWAP_AT_NS: u64 = 10_000_000_000;
const END_NS: u64 = 15_000_000_000;
/// The MAVLink id of SET_POSITION_TARGET_LOCAL_NED, the setpoint the
/// flight controller times its command loss on.
const SETPOINT_ID: u32 = 84;
/// Aviate's command-loss window.
const COMMAND_LOSS_NS: u64 = 500_000_000;

fn config() -> HostCoreConfig {
    let anchor = GeodeticPosition::new(LAT_DEG.to_radians(), LON_DEG.to_radians(), ALT_M);
    let mut mission = MissionConfig::new(String::new(), anchor, MISSION_CLOCK);
    mission.ownship_source = OwnshipSource::vehicle_estimate();
    mission.vehicle_class = VehicleClass::FixedWing;
    mission.cruise_height_m = 0.0;
    mission.cruise_mps = 55.0;
    mission.limits.max_horizontal_mps = 65.0;
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

fn first_plan() -> WaypointPlan {
    WaypointPlan::new(vec![
        PlannedWaypoint::new(LAT_DEG, LON_DEG + 0.03, ALT_M),
        PlannedWaypoint::new(LAT_DEG + 0.02, LON_DEG + 0.06, ALT_M),
    ])
}

fn second_plan() -> WaypointPlan {
    WaypointPlan::new(vec![
        PlannedWaypoint::new(LAT_DEG - 0.02, LON_DEG, ALT_M),
        PlannedWaypoint::new(LAT_DEG - 0.04, LON_DEG - 0.03, ALT_M),
    ])
}

/// The FC's datagrams of the capture, with their times.
fn from_fc() -> Vec<(u64, Vec<u8>)> {
    CAPTURE
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let at = parts.next()?.parse::<u64>().ok()?;
            (parts.next()? == "from_fc").then_some(())?;
            let text = parts.next()?;
            let bytes = (0..text.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok())
                .collect::<Option<Vec<u8>>>()?;
            Some((at, bytes))
        })
        .collect()
}

/// The MAVLink message id of one frame.
fn message_id(frame: &[u8]) -> Option<u32> {
    match frame {
        [0xFD, _, _, _, _, _, _, low, mid, high, ..] => {
            Some(u32::from(*low) | u32::from(*mid) << 8 | u32::from(*high) << 16)
        }
        [0xFE, _, _, _, _, id, ..] => Some(u32::from(*id)),
        _ => None,
    }
}

/// What one replay gives: the setpoint times, the events with their
/// times, what the second load returned, and the core.
struct Replay {
    setpoints: Vec<u64>,
    events: Vec<(u64, HostEvent)>,
    second_loaded: bool,
    core: HostCore,
}

/// Replays the capture with the first plan at 2 s and `second` at 10 s.
fn replay(second: WaypointPlan) -> Replay {
    let mut core = HostCore::new(config(), Instant::now()).unwrap();
    let mut setpoints = Vec::new();
    let mut events = Vec::new();
    let mut wake = MonotonicNanos::from_nanos(0);
    let mut second = Some(second);
    let mut second_loaded = false;
    let mut first_loaded = false;
    let mut run_to = |core: &mut HostCore, until: u64, wake: &mut MonotonicNanos| {
        while wake.as_nanos() <= until {
            let at = wake.as_nanos();
            *wake = core.tick(*wake).unwrap().at;
            for frame in core.drain_uplink() {
                if message_id(&frame) == Some(SETPOINT_ID) {
                    setpoints.push(at);
                }
            }
            events.extend(core.drain_events().into_iter().map(|event| (at, event)));
        }
    };
    for (at, datagram) in from_fc() {
        if !first_loaded && at >= FIRST_AT_NS {
            run_to(&mut core, FIRST_AT_NS, &mut wake);
            core.load_mission(first_plan()).unwrap();
            first_loaded = true;
        }
        if at >= SWAP_AT_NS
            && let Some(plan) = second.take()
        {
            run_to(&mut core, SWAP_AT_NS, &mut wake);
            second_loaded = core.load_mission(plan).is_ok();
        }
        run_to(&mut core, at, &mut wake);
        let now = MonotonicNanos::from_nanos(at);
        core.on_downlink(&datagram, now);
        core.on_uplink_reply(&datagram, now);
    }
    run_to(&mut core, END_NS, &mut wake);
    Replay {
        setpoints,
        events,
        second_loaded,
        core,
    }
}

/// The longest time between setpoints from `from_ns` on, and when it
/// started.
fn longest_gap(setpoints: &[u64], from_ns: u64) -> (u64, u64) {
    setpoints
        .iter()
        .filter(|at| **at >= from_ns)
        .copied()
        .collect::<Vec<u64>>()
        .windows(2)
        .map(|pair| (pair[1] - pair[0], pair[0]))
        .max()
        .unwrap_or((u64::MAX, 0))
}

#[test]
fn a_plan_loaded_in_flight_takes_over_without_a_gap_in_the_setpoints() {
    let Replay {
        setpoints,
        events,
        second_loaded,
        core,
    } = replay(second_plan());
    assert!(second_loaded);
    // The setpoints keep the mission cadence of 50 ms through the
    // handover: the flying plan frames until the transfer commits, and the
    // new plan does not arm a vehicle that is armed.
    let (gap, at) = longest_gap(&setpoints, FIRST_AT_NS + 1_000_000_000);
    assert!(
        gap <= 100_000_000,
        "the setpoints stopped for {} ms at {} ms",
        gap / 1_000_000,
        at / 1_000_000
    );
    assert!(gap < COMMAND_LOSS_NS);
    let last = setpoints.last().copied().unwrap_or_default();
    assert!(
        last >= END_NS - 100_000_000,
        "the setpoints stopped at {last} ns"
    );
    let took_over = events
        .iter()
        .find(|(_, event)| matches!(event, HostEvent::NewPlanFlying))
        .map(|(at, _)| *at)
        .expect("the new plan took over");
    assert!(
        took_over - SWAP_AT_NS < 200_000_000,
        "took over at {took_over} ns"
    );
    // The core flies the new plan: its engine is built, and its mission
    // runs from the takeover on.
    let state = core.status().mission_state;
    assert!(
        state.is_some() && state != Some(MissionState::Failed),
        "{:?}",
        core.status()
    );
    assert!(
        events.iter().any(|(at, event)| *at >= took_over
            && matches!(event, HostEvent::Mission(MissionEvent::ArmRequested { .. }))),
        "the new mission did not run: {events:?}"
    );
    assert_eq!(core.status().frames_rejected, 0, "{:?}", core.status());
}

#[test]
fn a_plan_that_does_not_build_leaves_the_flown_plan_flying() {
    let Replay {
        setpoints,
        events,
        second_loaded,
        ..
    } = replay(WaypointPlan::new(Vec::new()));
    assert!(!second_loaded, "an empty plan is refused");
    let (gap, at) = longest_gap(&setpoints, FIRST_AT_NS + 1_000_000_000);
    assert!(gap <= 100_000_000, "a {gap} ns gap at {at} ns");
    let last = setpoints.last().copied().unwrap_or_default();
    assert!(
        last >= END_NS - 100_000_000,
        "the setpoints stopped at {last} ns"
    );
    assert!(
        !events
            .iter()
            .any(|(_, event)| matches!(event, HostEvent::NewPlanFlying))
    );
}
