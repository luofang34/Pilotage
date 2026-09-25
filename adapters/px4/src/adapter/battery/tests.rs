#![allow(clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pilotage_adapter_api::{SourceRole, VehicleAdapter};
use pilotage_mavlink::codec::FcMessage;
use pilotage_mavlink::link::apply_messages_at;
use pilotage_mavlink::{BatteryReport, LinkState};
use pilotage_protocol::VehicleId;

use super::*;
use crate::adapter::Px4Adapter;
use crate::adapter::tests::{SOURCE, live_state};

fn report(received_at: Instant) -> BatteryReport {
    BatteryReport {
        instance: 0,
        remaining_percent: Some(73),
        voltage_mv: Some(16_200),
        current_ca: Some(1_250),
        energy_consumed_hj: Some(4_321),
        time_remaining_s: Some(900),
        sequence: 4,
        received_at,
    }
}

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
fn a_report_becomes_an_si_sample_under_the_fc_state_role() {
    let started = Instant::now();
    let sample = battery_sample(
        Some(report(Instant::now())),
        SourceIncarnation::new([7; 16]),
        started,
    )
    .expect("fresh report");
    assert_eq!(sample.remaining_fraction, Some(0.73));
    assert_eq!(sample.voltage_v, Some(16.2));
    assert_eq!(sample.current_a, Some(12.5));
    assert_eq!(sample.consumed_energy_j, Some(432_100.0));
    assert_eq!(sample.time_remaining_s, Some(900));
    assert_eq!(sample.stamp.role, SourceRole::FcState);
    assert_eq!(sample.stamp.sequence, 4);
    assert_eq!(sample.stamp.clock, MeasurementClock::HostMonotonic);
}

#[test]
fn a_stale_report_is_withheld() {
    let now = Instant::now();
    let Some(old) = now.checked_sub(WITHHOLD_AFTER + Duration::from_millis(1)) else {
        return;
    };
    let sample = battery_sample(Some(report(old)), SourceIncarnation::new([0; 16]), old);
    assert!(
        sample.is_none(),
        "a battery value past its freshness is not shown"
    );
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
