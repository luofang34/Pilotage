use std::time::Duration;

use pilotage_adapter_api::{
    BatterySample, MeasurementClock, MeasurementStamp, SourceIncarnation, SourceIntegrity,
    SourceRole,
};

use crate::{DumlFrame, RouteAddress};

// Three missing reports exhaust the observed 1 Hz battery stream.
const MAX_AGE: Duration = Duration::from_secs(3);
// This is a broad sanity bound for a two-cell pack, not an operating limit.
const VOLTAGE_MV: core::ops::RangeInclusive<u16> = 4_000..=10_000;
// Route 09 is the air link relay. Include the command to identify this stream
// without claiming that the relay is the physical flight controller.
const SOURCE_ID: u64 = 0x09_03_51;

#[derive(Clone, Copy)]
pub(super) struct BatteryReport {
    sequence: u16,
    remaining_percent: u8,
    voltage_mv: u16,
}

impl BatteryReport {
    pub(super) fn from_push(frame: &DumlFrame) -> Option<Self> {
        if frame.version() != 1
            || frame.sender() != RouteAddress(9)
            || frame.receiver() != RouteAddress(2)
            || frame.flags() != 0
            || frame.command_set() != 3
            || frame.command_id() != 0x51
        {
            return None;
        }
        let payload: &[u8; 30] = frame.payload().try_into().ok()?;
        let voltage_mv = u16::from_le_bytes([payload[24], payload[25]]);
        let remaining_percent = payload[26];
        if remaining_percent > 100 || !VOLTAGE_MV.contains(&voltage_mv) {
            return None;
        }
        Some(Self {
            sequence: frame.sequence(),
            remaining_percent,
            voltage_mv,
        })
    }
}

#[derive(Default)]
pub(super) struct BatteryObservations {
    latest: Option<(BatteryReport, Duration)>,
    last_sequence: Option<u16>,
    model_revision: Option<u64>,
    sequence: u32,
    published_sequence: u32,
}

impl BatteryObservations {
    pub(super) fn takeoff_deadline(&self, now: Duration, model_revision: u64) -> Option<Duration> {
        let (report, received) = self.latest?;
        let until = received.checked_add(MAX_AGE)?;
        // This is the host's experimental takeoff policy, not an FC limit.
        (self.model_revision == Some(model_revision)
            && report.remaining_percent >= 25
            && now >= received
            && now < until)
            .then_some(until)
    }

    pub(super) fn receive(
        &mut self,
        frame: &DumlFrame,
        now: Duration,
        model_revision: Option<u64>,
    ) {
        let Some(report) = BatteryReport::from_push(frame) else {
            return;
        };
        if let Some(previous) = self.last_sequence {
            let delta = report.sequence.wrapping_sub(previous);
            // A repeat or backwards packet cannot renew the receive time.
            if delta == 0 || delta >= 0x8000 {
                return;
            }
        }
        self.sequence = self.sequence.wrapping_add(1);
        self.last_sequence = Some(report.sequence);
        self.model_revision = model_revision;
        self.latest = Some((report, now));
    }

    pub(super) fn invalidate_model(&mut self) {
        self.latest = None;
        self.model_revision = None;
    }

    pub(super) fn sample(
        &mut self,
        now: Duration,
        connection_valid: bool,
        incarnation: SourceIncarnation,
    ) -> Option<BatterySample> {
        let (report, received) = self.latest?;
        if !connection_valid
            || self.sequence == self.published_sequence
            || !now.checked_sub(received).is_some_and(|age| age <= MAX_AGE)
        {
            return None;
        }
        self.published_sequence = self.sequence;
        Some(BatterySample {
            instance: 0,
            remaining_fraction: Some(f32::from(report.remaining_percent) / 100.0),
            voltage_v: Some(f32::from(report.voltage_mv) / 1_000.0),
            current_a: None,
            consumed_energy_j: None,
            time_remaining_s: None,
            stamp: MeasurementStamp {
                role: SourceRole::FcState,
                integrity: SourceIntegrity::ChecksummedOnly,
                source_id: SOURCE_ID,
                source_incarnation: incarnation,
                source_epoch: 0,
                sequence: self.sequence,
                // DUML supplies no verified acquisition clock for this report.
                acquired_at_ns: received.as_nanos().min(u128::from(u64::MAX)) as u64,
                clock: MeasurementClock::HostMonotonic,
            },
        })
    }
}
