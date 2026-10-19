//! The declination ArduPilot applies to its compass heading, read from
//! the FC's parameters.
//!
//! - `COMPASS_DEC` is the declination, radians, positive east.
//! - `COMPASS_AUTODEC` set means ArduPilot computes it from its own
//!   geomagnetic table at the vehicle's position; clear means the value
//!   was configured.

use std::time::{Duration, Instant};

use pilotage_adapter_api::{
    MagneticVariationSample, MagneticVariationSource, MeasurementClock, MeasurementStamp,
    SourceIntegrity, SourceRole,
};
use pilotage_mavlink::{LinkState, OutboundParam, ParamRequest};
use tokio::sync::mpsc;
use tracing::warn;

use super::params::number;

const COMPASS_DEC: &str = "COMPASS_DEC";
const COMPASS_AUTODEC: &str = "COMPASS_AUTODEC";
/// How often the parameters are read again: the automatic declination
/// changes as the vehicle moves.
const REFRESH: Duration = Duration::from_secs(5);
/// A declination unread for this long is no longer stated.
const WITHHOLD_AFTER: Duration = Duration::from_secs(15);

/// The FC's declination parameters and the sample they state.
#[derive(Debug)]
pub(super) struct DeclinationParams {
    lane: mpsc::Sender<OutboundParam>,
    system_id: u8,
    component_id: u8,
    last_request: Option<Instant>,
    last_report: Option<Instant>,
    sequence: u32,
}

impl DeclinationParams {
    pub(super) fn new(lane: mpsc::Sender<OutboundParam>, system_id: u8, component_id: u8) -> Self {
        Self {
            lane,
            system_id,
            component_id,
            last_request: None,
            last_report: None,
            sequence: 0,
        }
    }

    /// Advances the sample sequence on each new report and keeps the reads
    /// going.
    pub(super) fn maintain(&mut self, latest: &LinkState, now: Instant) {
        let received = latest
            .fc_reports
            .param(COMPASS_DEC)
            .map(|report| report.received_at);
        if received.is_some() && received != self.last_report {
            if self.last_report.is_some() {
                self.sequence = self.sequence.wrapping_add(1);
            }
            self.last_report = received;
        }
        if self
            .last_request
            .is_none_or(|at| now.duration_since(at) >= REFRESH)
        {
            self.last_request = Some(now);
            for name in [COMPASS_DEC, COMPASS_AUTODEC] {
                let outbound = OutboundParam {
                    request: ParamRequest::Read(name),
                    target_system: self.system_id,
                    target_component: self.component_id,
                };
                if self.lane.try_send(outbound).is_err() {
                    warn!(name, "ArduPilot parameter lane refused a declination read");
                }
            }
        }
    }

    /// The declination the FC applies, once both parameters are known and
    /// while the declination is current.
    pub(super) fn sample(&self, latest: &LinkState) -> Option<MagneticVariationSample> {
        let declination = latest
            .fc_reports
            .param(COMPASS_DEC)
            .filter(|report| report.received_at.elapsed() <= WITHHOLD_AFTER)?;
        let automatic = latest.fc_reports.param(COMPASS_AUTODEC)?;
        let source = if number(automatic.value) != 0.0 {
            MagneticVariationSource::FcModel
        } else {
            MagneticVariationSource::FcConfigured
        };
        let declination_rad = number(declination.value);
        if !declination_rad.is_finite() {
            return None;
        }
        Some(MagneticVariationSample {
            declination_rad,
            source,
            // ArduPilot names no table version over the link.
            model_id: None,
            model_epoch: None,
            stamp: MeasurementStamp {
                role: SourceRole::OperationalEstimate,
                integrity: SourceIntegrity::ChecksummedOnly,
                source_id: latest.source_id,
                source_incarnation: latest.source_incarnation,
                source_epoch: latest.source_epoch,
                sequence: self.sequence,
                acquired_at_ns: u64::try_from(
                    declination
                        .received_at
                        .saturating_duration_since(latest.started_at)
                        .as_nanos(),
                )
                .unwrap_or(u64::MAX),
                clock: MeasurementClock::HostMonotonic,
            },
        })
    }
}

#[cfg(test)]
mod tests;
