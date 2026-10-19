//! The heartbeat-derived arm state as a telemetry sample.

use std::time::Instant;

use pilotage_adapter_api::{
    FcCommandAck, FcStateSample, MeasurementClock, MeasurementStamp, SourceIncarnation,
    SourceIntegrity, SourceRole,
};

use crate::LinkState;

/// The latest FC arm report derived from telemetry heartbeats.
#[derive(Debug, Clone, Copy)]
pub struct ArmReport {
    /// Whether the FC reported itself armed.
    pub armed: bool,
    /// Wrapping count of heartbeats folded into this lane.
    pub sequence: u32,
    /// When the heartbeat was observed.
    pub acquired_at: Instant,
}

/// Folds each newly received heartbeat into a fresh [`ArmReport`], so a
/// silent FC honestly ages into staleness instead of replaying the last
/// state forever.
#[derive(Debug, Default)]
pub struct ArmTracker {
    report: Option<ArmReport>,
    last_seen_heartbeat: Option<Instant>,
}

impl ArmTracker {
    /// Re-acquires the report when the cache holds a heartbeat this
    /// tracker has not seen (fresh stamp, advanced sequence).
    pub fn observe(&mut self, latest: &LinkState) {
        let Some((armed, heartbeat_at)) = latest.heartbeat_armed.zip(latest.last_heartbeat) else {
            return;
        };
        if self.last_seen_heartbeat == Some(heartbeat_at) {
            return;
        }
        self.last_seen_heartbeat = Some(heartbeat_at);
        let sequence = self
            .report
            .map_or(0, |report| report.sequence.wrapping_add(1));
        self.report = Some(ArmReport {
            armed,
            sequence,
            acquired_at: Instant::now(),
        });
    }

    /// The latest report, if a heartbeat was ever observed.
    #[must_use]
    pub fn report(&self) -> Option<ArmReport> {
        self.report
    }
}

/// The heartbeat-derived FC state sample: arm reported by the FC's own
/// telemetry, stamped under the FC-state source role.
#[must_use]
pub fn fc_state_sample(
    report: Option<ArmReport>,
    last_command: Option<FcCommandAck>,
    incarnation: SourceIncarnation,
    started_at: Instant,
) -> Option<FcStateSample> {
    let report = report?;
    let acquired = report
        .acquired_at
        .checked_duration_since(started_at)
        .unwrap_or_default();
    Some(FcStateSample {
        arm_state: if report.armed { 2 } else { 1 },
        // The FC's verdict on the most recent commanded arm/disarm rides
        // the same report provenance: enactment truth the operator needs
        // when "command accepted" and "vehicle armed" disagree.
        last_command,
        stamp: MeasurementStamp {
            role: SourceRole::FcState,
            // MAVLink frames are CRC-checked but unsigned: checksummed,
            // never authenticated.
            integrity: SourceIntegrity::ChecksummedOnly,
            source_id: 1,
            source_incarnation: incarnation,
            // A gateway-generated attachment identity cannot observe an
            // FC restart; a source-issued boot identity replaces this
            // constant once the FC publishes one.
            source_epoch: 1,
            sequence: report.sequence,
            acquired_at_ns: u64::try_from(acquired.as_nanos()).unwrap_or(u64::MAX),
            clock: MeasurementClock::HostMonotonic,
        },
    })
}

#[cfg(test)]
mod tests;
