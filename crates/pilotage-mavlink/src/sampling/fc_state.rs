//! The heartbeat-derived arm state as a telemetry sample.

use std::time::Instant;

use pilotage_adapter_api::{
    FcCommandAck, FcStateSample, LandedStateSample, MeasurementClock, MeasurementStamp,
    SourceIntegrity, SourceRole,
};
use pilotage_protocol::LandedState;

use crate::LinkState;

/// The latest FC arm report derived from telemetry heartbeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmReport {
    /// Whether the FC reported itself armed.
    pub armed: bool,
    /// Selected FC identity and the heartbeat's receiver-clock acquisition.
    pub stamp: MeasurementStamp,
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
            self.report = None;
            self.last_seen_heartbeat = None;
            return;
        };
        let current = self.report.filter(|report| {
            report.stamp.source_id == latest.source_id
                && report.stamp.source_incarnation == latest.source_incarnation
                && report.stamp.source_epoch == latest.source_epoch
        });
        if current.is_some() && self.last_seen_heartbeat == Some(heartbeat_at) {
            return;
        }
        self.last_seen_heartbeat = Some(heartbeat_at);
        let sequence = current.map_or(0, |report| report.stamp.sequence.wrapping_add(1));
        let acquired = heartbeat_at.saturating_duration_since(latest.started_at);
        self.report = Some(ArmReport {
            armed,
            stamp: MeasurementStamp {
                role: SourceRole::FcState,
                integrity: SourceIntegrity::ChecksummedOnly,
                source_id: latest.source_id,
                source_incarnation: latest.source_incarnation,
                source_epoch: latest.source_epoch,
                sequence,
                acquired_at_ns: u64::try_from(acquired.as_nanos()).unwrap_or(u64::MAX),
                clock: MeasurementClock::HostMonotonic,
            },
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
    landed_state: Option<LandedStateSample>,
) -> Option<FcStateSample> {
    let report = report?;
    Some(FcStateSample {
        landing_approach: None,
        arm_state: if report.armed { 2 } else { 1 },
        // The FC's verdict on the most recent commanded arm/disarm rides
        // the same report provenance: enactment truth the operator needs
        // when "command accepted" and "vehicle armed" disagree.
        last_command,
        stamp: Some(report.stamp),
        landed_state,
        motor_state: None,
    })
}

/// Samples a current landed-state report without changing its acquisition stamp.
#[must_use]
pub fn landed_state_sample(latest: &LinkState, now: Instant) -> Option<LandedStateSample> {
    let report = latest.landed_state_cached(now, super::WITHHOLD_AFTER)?;
    let state = match report.state {
        crate::LandedState::Undefined => LandedState::Unknown,
        crate::LandedState::OnGround => LandedState::OnGround,
        crate::LandedState::InAir => LandedState::InAir,
        crate::LandedState::Takeoff => LandedState::TakingOff,
        crate::LandedState::Landing => LandedState::Landing,
    };
    Some(LandedStateSample {
        state,
        stamp: report.stamp,
    })
}

#[cfg(test)]
mod tests;
