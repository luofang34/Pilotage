#![allow(clippy::expect_used, clippy::panic)]

//! The battery lane through the adapter: it reaches telemetry with or
//! without an estimate, and only the primary battery is kept.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use pilotage_adapter_api::VehicleAdapter;
use pilotage_mavlink::LinkState;
use pilotage_mavlink::codec::FcMessage;
use pilotage_mavlink::link::apply_messages_at;
use pilotage_protocol::VehicleId;

use super::{SOURCE, live_state};
use crate::adapter::Px4Adapter;

fn battery_message(remaining_percent: Option<u8>) -> FcMessage {
    FcMessage::BatteryStatus {
        instance: 0,
        remaining_percent,
        voltage_mv: Some(16_200),
        current_ca: Some(1_250),
        energy_consumed_hj: None,
        time_remaining_s: None,
    }
}

#[test]
fn a_battery_report_reaches_telemetry_without_an_estimate() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), state.clone());
    apply_messages_at(
        &state,
        &[(SOURCE, battery_message(Some(50)))],
        0,
        0,
        Instant::now(),
    );
    let batch = adapter.sample_telemetry();
    let [sample] = batch.samples.as_slice() else {
        panic!("one sample carries the battery report alone");
    };
    assert!(sample.pose.is_none() && sample.avionics.is_none());
    let battery = sample.battery.expect("battery sample");
    assert_eq!(battery.remaining_fraction, Some(0.5));
    assert_eq!(battery.stamp.sequence, 0);

    apply_messages_at(
        &state,
        &[(SOURCE, battery_message(None))],
        0,
        0,
        Instant::now(),
    );
    let battery = adapter.sample_telemetry().samples[0]
        .battery
        .expect("battery sample");
    assert_eq!(battery.remaining_fraction, None, "unknown stays unknown");
    assert_eq!(
        battery.stamp.sequence, 1,
        "each report advances the sequence"
    );
}

#[test]
fn a_battery_report_rides_beside_the_estimate() {
    let state = live_state();
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), state.clone());
    apply_messages_at(
        &state,
        &[(SOURCE, battery_message(Some(80)))],
        0,
        0,
        Instant::now(),
    );
    let batch = adapter.sample_telemetry();
    assert!(!batch.samples.is_empty());
    for sample in &batch.samples {
        assert!(sample.avionics.is_some());
        assert_eq!(sample.battery.and_then(|b| b.remaining_fraction), Some(0.8));
    }
}

#[test]
fn only_the_primary_battery_is_kept() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    let second = FcMessage::BatteryStatus {
        instance: 1,
        remaining_percent: Some(10),
        voltage_mv: None,
        current_ca: None,
        energy_consumed_hj: None,
        time_remaining_s: None,
    };
    let now = Instant::now();
    apply_messages_at(
        &state,
        &[(SOURCE, battery_message(Some(90))), (SOURCE, second)],
        0,
        0,
        now,
    );
    let latest = state.lock().expect("state");
    let kept = latest.battery.expect("primary kept");
    assert_eq!((kept.instance, kept.remaining_percent), (0, Some(90)));
    assert_eq!(latest.other_battery_reports, 1);
}
