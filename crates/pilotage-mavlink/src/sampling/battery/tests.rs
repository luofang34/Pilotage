#![allow(clippy::expect_used)]

use std::time::{Duration, Instant};

use pilotage_adapter_api::{
    MeasurementClock, MeasurementStamp, SourceIncarnation, SourceIntegrity, SourceRole,
};

use super::{WITHHOLD_AFTER, battery_sample};
use crate::BatteryReport;
use crate::sampling::{ArmReport, fc_state_sample};

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
    let fc_state = fc_state_sample(
        Some(ArmReport {
            armed: false,
            stamp: MeasurementStamp {
                role: SourceRole::FcState,
                integrity: SourceIntegrity::ChecksummedOnly,
                source_id: 1,
                source_incarnation: SourceIncarnation::new([7; 16]),
                source_epoch: 1,
                sequence: 0,
                acquired_at_ns: 0,
                clock: MeasurementClock::HostMonotonic,
            },
        }),
        None,
        None,
    )
    .expect("fc state");
    assert_ne!(
        sample.stamp.source_id, fc_state.stamp.source_id,
        "battery and FC state are separate lanes of one source"
    );
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
