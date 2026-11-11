#![cfg(test)]

//! The core as an embedding host drives it: built, loaded with a waypoint
//! plan, and ticked at the times it asks for, with no flight controller
//! on the other end yet.

use std::time::Duration;

use navigate_contract::{GeodeticPosition, MonotonicNanos};
use pilotage_adapter_api::SourceIncarnation;
use pilotage_adapter_aviate::{AviateAirframe, EmbeddedIncarnations};
use pilotage_control_feel::{FlightFeelProfile, ValidatedFlightFeelProfile};
use pilotage_host_core::{HostCore, HostCoreConfig, HostCoreError, HostEvent, MISSION_CLOCK};
use pilotage_mavlink::link::LinkConfig;
use pilotage_mission::{MissionConfig, OwnshipSource, PlannedWaypoint, WaypointPlan};
use web_time::Instant;

fn config() -> HostCoreConfig {
    let anchor = GeodeticPosition::new(47.0_f64.to_radians(), 8.0_f64.to_radians(), 500.0);
    let mut mission = MissionConfig::new(String::new(), anchor, MISSION_CLOCK);
    mission.ownship_source = OwnshipSource::vehicle_estimate();
    HostCoreConfig {
        link: LinkConfig::default(),
        airframe: AviateAirframe::Multirotor(Box::new(
            ValidatedFlightFeelProfile::new(FlightFeelProfile::legacy_compatibility()).unwrap(),
        )),
        fc_origin_height_m: Some(500.0),
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
        PlannedWaypoint::new(47.0006, 8.0, 512.0),
        PlannedWaypoint::new(47.0006, 8.0008, 518.0),
    ])
}

/// Runs a core for `seconds` at the times it asks for and returns the
/// uplink bytes it produced.
fn run(seconds: u64) -> (HostCore, Vec<Vec<u8>>) {
    let mut core = HostCore::new(config(), Instant::now()).unwrap();
    core.load_mission(plan()).unwrap();
    let end = Duration::from_secs(seconds).as_nanos() as u64;
    let mut now = MonotonicNanos::from_nanos(0);
    let mut uplink = Vec::new();
    while now.as_nanos() < end {
        let wake = core.tick(now).unwrap();
        uplink.extend(core.drain_uplink());
        assert!(wake.at > now, "the core asks to be called later");
        now = wake.at;
    }
    (core, uplink)
}

#[test]
fn a_loaded_mission_opens_its_session_and_builds_its_engine() {
    let (mut core, _) = run(2);
    assert!(
        core.status().mission_state.is_some(),
        "the welcome and the capabilities built the mission engine"
    );
    // With no flight controller the mission has no solution to fly.
    assert_eq!(
        core.status().mission_state,
        Some(pilotage_mission::MissionState::AwaitSolution)
    );
    // The engine says that it waits before the arm, and the holder's
    // frames meet an adapter with no measurement yet: nothing else.
    assert!(core.drain_events().iter().all(|event| matches!(
        event,
        HostEvent::Mission(pilotage_mission::MissionEvent::StartWithheld { .. })
            | HostEvent::FrameRejected
    )));
    assert!(
        core.status().frames_sent > 0,
        "the holder frames while it waits"
    );
    assert_eq!(core.declared_start(), config().mission.anchor);
}

#[test]
fn the_same_inputs_give_the_same_bytes() {
    let (first, first_uplink) = run(5);
    let (second, second_uplink) = run(5);
    assert_eq!(first_uplink, second_uplink);
    assert_eq!(first.status(), second.status());
}

#[test]
fn time_does_not_go_back() {
    let mut core = HostCore::new(config(), Instant::now()).unwrap();
    core.tick(MonotonicNanos::from_nanos(1_000)).unwrap();
    assert!(matches!(
        core.tick(MonotonicNanos::from_nanos(10)),
        Err(HostCoreError::TimeWentBack { .. })
    ));
}

#[test]
fn a_fixed_wing_core_builds_with_its_envelope_and_origin() {
    let mut config = config();
    config.airframe = AviateAirframe::FixedWing(
        pilotage_control_feel::ValidatedFixedWingEnvelope::new(
            pilotage_control_feel::FixedWingEnvelope::C172,
        )
        .unwrap(),
    );
    config.mission.vehicle_class = pilotage_protocol::VehicleClass::FixedWing;
    let mut core = HostCore::new(config, Instant::now()).unwrap();
    core.load_mission(plan()).unwrap();
    core.tick(MonotonicNanos::from_nanos(0)).unwrap();
    assert!(
        core.status().mission_state.is_some(),
        "the C172 mission builds"
    );
}

#[test]
fn the_declared_start_is_the_navigators_initial_state() {
    let mut core = HostCore::new(config(), Instant::now()).unwrap();
    assert_eq!(core.navigator_initial_state(), None, "no mission yet");
    let declared = pilotage_mission::DeclaredStart {
        position: None,
        height: None,
        heading: Some(pilotage_mission::DeclaredHeading {
            heading_rad: 1.2,
            sigma_rad: 0.05,
        }),
        speed: None,
        declared_at: MonotonicNanos::from_nanos(0),
    };
    let mut config = config();
    config.mission.declared_start = Some(declared);
    let mut core_with = HostCore::new(config, Instant::now()).unwrap();
    core_with.load_mission(plan()).unwrap();
    core_with.tick(MonotonicNanos::from_nanos(0)).unwrap();
    assert_eq!(core_with.navigator_initial_state(), Some(declared));
    core.load_mission(plan()).unwrap();
    core.tick(MonotonicNanos::from_nanos(0)).unwrap();
    assert_eq!(core.navigator_initial_state(), None, "the plan states none");
}
