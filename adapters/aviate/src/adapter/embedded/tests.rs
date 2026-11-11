use std::time::Duration;

use pilotage_adapter_api::{LinkLossPolicy, SourceIncarnation, VehicleAdapter};
use pilotage_control_feel::{FlightFeelProfile, ValidatedFlightFeelProfile};
use pilotage_mavlink::codec::{encode_autopilot_heartbeat, parse_datagram};
use pilotage_mavlink::link::LinkConfig;
use pilotage_protocol::{ScopeId, VehicleId};
use web_time::Instant;

use super::EmbeddedIncarnations;
use crate::adapter::{AviateAdapter, FLIGHT_SCOPE};

fn embedded(start: Instant) -> AviateAdapter {
    let feel = ValidatedFlightFeelProfile::new(FlightFeelProfile::legacy_compatibility()).unwrap();
    let incarnations = EmbeddedIncarnations {
        estimate: SourceIncarnation::new([1; 16]),
        fc_state: SourceIncarnation::new([2; 16]),
    };
    AviateAdapter::embedded(
        VehicleId::new(1),
        LinkConfig::default(),
        feel,
        incarnations,
        start,
    )
    .unwrap()
}

#[test]
fn an_fc_reply_is_stamped_at_the_time_the_host_set() {
    let start = Instant::now();
    let mut adapter = embedded(start);
    adapter.set_now(start + Duration::from_secs(2));
    assert!(adapter.on_uplink_reply(&encode_autopilot_heartbeat(0, true, 0, 1, 1)));
    adapter.sample_telemetry();
    let report = adapter.arm.expect("the heartbeat reports the arm state");
    assert!(report.armed);
    assert_eq!(report.acquired_at, start + Duration::from_secs(2));
}

#[test]
fn a_reply_from_another_fc_is_ignored() {
    let start = Instant::now();
    let mut adapter = embedded(start);
    assert!(adapter.on_uplink_reply(&encode_autopilot_heartbeat(0, true, 0, 7, 1)));
    adapter.sample_telemetry();
    assert!(adapter.arm.is_none());
}

#[test]
fn a_link_loss_neutral_is_a_frame_for_the_host_to_send() {
    let start = Instant::now();
    let mut adapter = embedded(start);
    assert!(adapter.drain_uplink().is_empty());
    adapter
        .set_link_loss_policy(
            VehicleId::new(1),
            &ScopeId::new(FLIGHT_SCOPE),
            Some(LinkLossPolicy::Neutralize),
        )
        .unwrap();
    let frames = adapter.drain_uplink();
    assert_eq!(frames.len(), 1, "one neutral setpoint");
    let mut messages = Vec::new();
    let stats = parse_datagram(&frames[0], &mut messages);
    assert_eq!(stats.crc_failures, 0, "a whole MAVLink frame");
    assert!(adapter.drain_uplink().is_empty(), "drained once");
}

/// The host's time is far from the process clock, so any stamp that came
/// from a clock read instead of `set_now` would differ from it.
#[test]
fn the_embedded_path_takes_every_time_from_the_host() {
    let start = Instant::now() + Duration::from_secs(10_000);
    let mut adapter = embedded(start);
    adapter.set_now(start + Duration::from_secs(3));
    adapter.on_downlink(&encode_autopilot_heartbeat(0, false, 0, 1, 1));
    assert!(adapter.on_uplink_reply(&encode_autopilot_heartbeat(1, false, 0, 1, 1)));
    adapter.sample_telemetry();
    let state = adapter.estimate.as_ref().unwrap().state.lock().unwrap();
    assert_eq!(state.started_at, start);
    assert_eq!(state.last_heartbeat, Some(start + Duration::from_secs(3)));
    assert_eq!(adapter.started_at, start);
    assert_eq!(
        adapter.arm.map(|report| report.acquired_at),
        Some(start + Duration::from_secs(3))
    );
}
