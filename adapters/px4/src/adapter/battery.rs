//! The battery report as a telemetry sample: wire units to SI units, under
//! the FC-state role, withheld once stale.

use pilotage_adapter_api::{
    BatterySample, MeasurementClock, MeasurementStamp, SourceIncarnation, SourceIntegrity,
    SourceRole,
};
use pilotage_mavlink::BatteryReport;

use super::WITHHOLD_AFTER;

/// Source id of the battery lane. Each lane from this flight controller has
/// its own id (FC state 1, gimbal device 2), so no consumer that tracks
/// sequences by role and source can read two lanes as one stream.
pub(super) const BATTERY_SOURCE_ID: u64 = 3;

/// The latest battery report as a sample, or `None` when no report is
/// fresh. The flight controller is the only author, so the sample has the
/// FC-state role. BATTERY_STATUS has no time of its own, so the
/// receive time is the acquisition time.
pub(super) fn battery_sample(
    report: Option<BatteryReport>,
    incarnation: SourceIncarnation,
    started_at: std::time::Instant,
) -> Option<BatterySample> {
    let report = report.filter(|r| r.received_at.elapsed() <= WITHHOLD_AFTER)?;
    let acquired = report
        .received_at
        .checked_duration_since(started_at)
        .unwrap_or_default();
    Some(BatterySample {
        instance: u32::from(report.instance),
        remaining_fraction: report.remaining_percent.map(|p| f32::from(p) / 100.0),
        voltage_v: report.voltage_mv.map(|mv| mv as f32 / 1_000.0),
        current_a: report.current_ca.map(|ca| f32::from(ca) / 100.0),
        consumed_energy_j: report.energy_consumed_hj.map(|hj| hj as f32 * 100.0),
        time_remaining_s: report.time_remaining_s,
        stamp: MeasurementStamp {
            role: SourceRole::FcState,
            // MAVLink frames are CRC-checked but unsigned.
            integrity: SourceIntegrity::ChecksummedOnly,
            source_id: BATTERY_SOURCE_ID,
            source_incarnation: incarnation,
            source_epoch: 1,
            sequence: report.sequence,
            acquired_at_ns: u64::try_from(acquired.as_nanos()).unwrap_or(u64::MAX),
            clock: MeasurementClock::HostMonotonic,
        },
    })
}

#[cfg(test)]
mod tests;
