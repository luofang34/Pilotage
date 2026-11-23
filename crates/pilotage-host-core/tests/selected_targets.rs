#![cfg(test)]

//! The targets an operator selects, through the core: the core against
//! the traffic of Aviate's C172 board (the `aviate_link` capture), with a
//! plan loaded at 2 s, a heading, an altitude and an airspeed selected at
//! 5 s, and a second plan loaded at 10 s. The setpoints the flight
//! controller receives must carry the selection, and the selection must
//! stay in effect when the second plan takes over.

use navigate_contract::{GeodeticPosition, MonotonicNanos};
use pilotage_adapter_api::SourceIncarnation;
use pilotage_adapter_aviate::{AviateAirframe, EmbeddedIncarnations};
use pilotage_control_feel::{FixedWingEnvelope, ValidatedFixedWingEnvelope};
use pilotage_host_core::{
    HostCore, HostCoreConfig, HostCoreError, HostEvent, MISSION_CLOCK, SelectError, SelectedTargets,
};
use pilotage_mavlink::codec::{HEADING_ALTITUDE_TYPE_MASK, MAV_CMD_DO_CHANGE_SPEED};
use pilotage_mavlink::link::LinkConfig;
use pilotage_mission::{
    LateralMode, MissionConfig, MissionEvent, OwnshipSource, PlannedWaypoint, WaypointPlan,
};
use pilotage_protocol::VehicleClass;
use web_time::Instant;

const CAPTURE: &str = include_str!("fixtures/aviate-c172-link-15s.txt");
const LAT_DEG: f64 = 41.7;
const LON_DEG: f64 = -1.07;
const ALT_M: f64 = 2050.0;
const FIRST_AT_NS: u64 = 2_000_000_000;
const SELECT_AT_NS: u64 = 5_000_000_000;
const SWAP_AT_NS: u64 = 10_000_000_000;
const END_NS: u64 = 15_000_000_000;
/// The MAVLink id of SET_POSITION_TARGET_LOCAL_NED.
const SETPOINT_ID: u32 = 84;
/// The MAVLink id of COMMAND_LONG.
const COMMAND_LONG_ID: u32 = 76;
/// Where the MAVLink 2 payload starts in a frame.
const PAYLOAD_AT: usize = 10;

fn selection() -> SelectedTargets {
    SelectedTargets {
        heading_true_rad: Some(2.0),
        altitude_m: Some(ALT_M + 150.0),
        airspeed_mps: Some(60.0),
    }
}

fn config(vehicle_class: VehicleClass) -> HostCoreConfig {
    let anchor = GeodeticPosition::new(LAT_DEG.to_radians(), LON_DEG.to_radians(), ALT_M);
    let mut mission = MissionConfig::new(String::new(), anchor, MISSION_CLOCK);
    mission.ownship_source = OwnshipSource::vehicle_estimate();
    mission.vehicle_class = vehicle_class;
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

fn f32_at(frame: &[u8], payload_offset: usize) -> Option<f32> {
    let at = PAYLOAD_AT + payload_offset;
    Some(f32::from_le_bytes(frame.get(at..at + 4)?.try_into().ok()?))
}

/// The down position and the true heading of a heading and altitude
/// setpoint; `None` for another frame.
fn heading_altitude(frame: &[u8]) -> Option<(f32, f32)> {
    let [0xFD, _, _, _, _, _, _, low, mid, high, ..] = frame else {
        return None;
    };
    let id = u32::from(*low) | u32::from(*mid) << 8 | u32::from(*high) << 16;
    let mask = frame.get(PAYLOAD_AT + 48..PAYLOAD_AT + 50)?;
    (id == SETPOINT_ID && mask == HEADING_ALTITUDE_TYPE_MASK.to_le_bytes())
        .then_some((f32_at(frame, 12)?, f32_at(frame, 40)?))
}

/// The airspeed of a command that sets the airspeed to hold; `None` for
/// another frame.
fn airspeed_command(frame: &[u8]) -> Option<f32> {
    let [0xFD, _, _, _, _, _, _, low, mid, high, ..] = frame else {
        return None;
    };
    let id = u32::from(*low) | u32::from(*mid) << 8 | u32::from(*high) << 16;
    let command = frame.get(PAYLOAD_AT + 28..PAYLOAD_AT + 30)?;
    (id == COMMAND_LONG_ID && command == MAV_CMD_DO_CHANGE_SPEED.to_le_bytes())
        .then_some(f32_at(frame, 4)?)
}

/// What one replay gives: each heading and altitude setpoint with its
/// time, each commanded airspeed with its time, the events with their
/// times, and the core.
struct Replay {
    setpoints: Vec<(u64, f32, f32)>,
    airspeeds: Vec<(u64, f32)>,
    events: Vec<(u64, HostEvent)>,
    core: HostCore,
}

/// Replays the capture with the first plan at 2 s, `selected` at 5 s, and
/// the second plan at 10 s.
fn replay(selected: Option<SelectedTargets>) -> Replay {
    let mut core = HostCore::new(config(VehicleClass::FixedWing), Instant::now()).unwrap();
    let mut setpoints = Vec::new();
    let mut airspeeds = Vec::new();
    let mut events = Vec::new();
    let mut wake = MonotonicNanos::from_nanos(0);
    let mut pending: Vec<u64> = vec![FIRST_AT_NS, SELECT_AT_NS, SWAP_AT_NS];
    let mut run_to = |core: &mut HostCore, until: u64, wake: &mut MonotonicNanos| {
        while wake.as_nanos() <= until {
            let at = wake.as_nanos();
            *wake = core.tick(*wake).unwrap().at;
            for frame in core.drain_uplink() {
                if let Some((down_m, heading_rad)) = heading_altitude(&frame) {
                    setpoints.push((at, down_m, heading_rad));
                }
                if let Some(airspeed_mps) = airspeed_command(&frame) {
                    airspeeds.push((at, airspeed_mps));
                }
            }
            events.extend(core.drain_events().into_iter().map(|event| (at, event)));
        }
    };
    for (at, datagram) in from_fc() {
        while let Some(due) = pending.first().copied().filter(|due| at >= *due) {
            pending.remove(0);
            run_to(&mut core, due, &mut wake);
            if due == FIRST_AT_NS {
                core.load_mission(first_plan()).unwrap();
            } else if due == SWAP_AT_NS {
                core.load_mission(second_plan()).unwrap();
            } else if let Some(selected) = selected {
                core.select_targets(selected).unwrap();
            }
        }
        run_to(&mut core, at, &mut wake);
        let now = MonotonicNanos::from_nanos(at);
        core.on_downlink(&datagram, now);
        core.on_uplink_reply(&datagram, now);
    }
    run_to(&mut core, END_NS, &mut wake);
    Replay {
        setpoints,
        airspeeds,
        events,
        core,
    }
}

/// Whether a setpoint carries the selected heading and altitude.
fn carries_selection(down_m: f32, heading_rad: f32) -> bool {
    (down_m + 150.0).abs() < 1e-3 && (heading_rad - 2.0).abs() < 1e-6
}

#[test]
fn the_selected_targets_reach_the_setpoints_and_stay_when_a_new_plan_takes_over() {
    let Replay {
        setpoints,
        airspeeds,
        events,
        core,
    } = replay(Some(selection()));
    let reported = events
        .iter()
        .find(|(_, event)| {
            matches!(event, HostEvent::Mission(MissionEvent::TargetsSelected { targets })
                if *targets == selection())
        })
        .map(|(at, _)| *at)
        .expect("the mission reported the selection");
    assert!(reported - SELECT_AT_NS <= 100_000_000, "at {reported} ns");
    // From the selection on, each heading and altitude setpoint is the
    // selected one, through the handover and to the end of the run.
    let selected: Vec<_> = setpoints
        .iter()
        .filter(|(at, ..)| *at >= reported)
        .collect();
    assert!(selected.len() > 100, "{} setpoints", selected.len());
    for (at, down_m, heading_rad) in &selected {
        assert!(
            carries_selection(*down_m, *heading_rad),
            "{down_m} m down on {heading_rad} rad at {at} ns"
        );
    }
    assert!(
        airspeeds
            .iter()
            .any(|(at, airspeed_mps)| *at >= reported && *airspeed_mps == 60.0),
        "the selected airspeed was commanded: {airspeeds:?}"
    );
    let took_over = events
        .iter()
        .find(|(_, event)| matches!(event, HostEvent::NewPlanFlying))
        .map(|(at, _)| *at)
        .expect("the new plan took over");
    let after: Vec<_> = selected
        .iter()
        .filter(|(at, ..)| *at >= took_over)
        .collect();
    assert!(
        after.len() > 50,
        "{} setpoints of the new plan",
        after.len()
    );
    let last = selected.last().map(|(at, ..)| *at).unwrap_or_default();
    assert!(last >= END_NS - 100_000_000, "the last one at {last} ns");
    // The new plan's principal frames its arm before it flies, so the
    // selected setpoints stop for that time and no longer.
    let times: Vec<u64> = selected.iter().map(|(at, ..)| *at).collect();
    let (gap, gap_at) = times
        .windows(2)
        .map(|pair| (pair[1] - pair[0], pair[0]))
        .max()
        .unwrap_or((u64::MAX, 0));
    assert!(
        gap <= 200_000_000,
        "the selected setpoints stopped for {gap} ns at {gap_at} ns"
    );
    assert_eq!(core.selected_targets(), selection());
    let navigation = core.status().navigation.as_ref().expect("an engine");
    assert_eq!(navigation.selected, selection());
    assert_eq!(navigation.lateral.mode, LateralMode::HeadingSelect);
    assert_eq!(core.status().frames_rejected, 0, "{:?}", core.status());
}

#[test]
fn without_a_selection_no_setpoint_carries_one() {
    let Replay {
        setpoints,
        airspeeds,
        core,
        ..
    } = replay(None);
    assert!(
        !setpoints
            .iter()
            .any(|(_, down_m, heading_rad)| carries_selection(*down_m, *heading_rad))
    );
    assert!(
        !airspeeds
            .iter()
            .any(|(_, airspeed_mps)| *airspeed_mps == 60.0)
    );
    assert_eq!(core.selected_targets(), SelectedTargets::default());
}

#[test]
fn the_core_refuses_a_selection_it_cannot_fly() {
    let mut core = HostCore::new(config(VehicleClass::FixedWing), Instant::now()).unwrap();
    assert!(matches!(
        core.select_targets(selection()),
        Err(HostCoreError::NoMission { .. })
    ));
    core.load_mission(first_plan()).unwrap();
    core.select_targets(selection()).unwrap();
    let not_a_number = SelectedTargets {
        heading_true_rad: Some(f64::NAN),
        ..SelectedTargets::default()
    };
    assert!(matches!(
        core.select_targets(not_a_number),
        Err(HostCoreError::Select {
            source: SelectError::NotFinite { .. }
        })
    ));
    assert_eq!(core.selected_targets(), selection(), "the selection stands");

    let mut core = HostCore::new(config(VehicleClass::Multicopter), Instant::now()).unwrap();
    core.load_mission(first_plan()).unwrap();
    assert!(matches!(
        core.select_targets(selection()),
        Err(HostCoreError::Select {
            source: SelectError::VehicleCanStop
        })
    ));
    core.select_targets(SelectedTargets::default()).unwrap();
}
