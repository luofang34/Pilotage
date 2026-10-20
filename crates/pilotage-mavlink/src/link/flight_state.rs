//! Selected flight-controller reports for observed flight procedures.

use std::time::{Duration, Instant};

use pilotage_adapter_api::{MeasurementClock, MeasurementStamp, SourceIntegrity, SourceRole};

use super::LinkState;
use crate::codec::LandedState;

/// Landed detector report with the receiver clock as its acquisition clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LandedStateReport {
    /// State stated by the flight controller.
    pub state: LandedState,
    /// Source identity, epoch, receive sequence, and receiver-clock acquisition.
    pub stamp: MeasurementStamp,
    /// Instant at which this report arrived.
    pub received_at: Instant,
}

/// Altitude above home, separate from local NED position and geodetic height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RelativeAltitudeReport {
    /// Altitude above the flight controller's home position, in metres.
    pub altitude_m: f32,
    /// Milliseconds since flight-controller boot.
    pub time_boot_ms: u32,
    /// Source identity, epoch, sequence, and boot-clock acquisition.
    pub stamp: MeasurementStamp,
    /// Instant at which this report arrived.
    pub received_at: Instant,
}

impl LinkState {
    /// Returns a landed report from this epoch within the receive-age bound.
    ///
    /// A pending clock reset makes flight-procedure evidence unavailable.
    #[must_use]
    pub fn landed_state_cached(
        &self,
        now: Instant,
        maximum_age: Duration,
    ) -> Option<LandedStateReport> {
        self.fc_reports.landed_state.filter(|report| {
            self.flight_report_is_current(report.stamp, report.received_at, now, maximum_age)
        })
    }

    /// Returns home-relative altitude from this epoch within the receive-age bound.
    ///
    /// The value does not authorize navigation or a local-position estimate.
    #[must_use]
    pub fn relative_altitude_cached(
        &self,
        now: Instant,
        maximum_age: Duration,
    ) -> Option<RelativeAltitudeReport> {
        self.fc_reports.relative_altitude.filter(|report| {
            self.flight_report_is_current(report.stamp, report.received_at, now, maximum_age)
        })
    }

    fn flight_report_is_current(
        &self,
        stamp: MeasurementStamp,
        received_at: Instant,
        now: Instant,
        maximum_age: Duration,
    ) -> bool {
        self.pending_reset.is_none()
            && stamp.source_id == self.source_id
            && stamp.source_incarnation == self.source_incarnation
            && stamp.source_epoch == self.source_epoch
            && now
                .checked_duration_since(received_at)
                .is_some_and(|age| age <= maximum_age)
    }
}

pub(super) fn apply_landed_state(latest: &mut LinkState, state: LandedState, now: Instant) {
    let sequence = latest
        .fc_reports
        .landed_state
        .map_or(0, |report| report.stamp.sequence.wrapping_add(1));
    let acquired_at_ns = u64::try_from(now.saturating_duration_since(latest.started_at).as_nanos())
        .unwrap_or(u64::MAX);
    latest.fc_reports.landed_state = Some(LandedStateReport {
        state,
        stamp: MeasurementStamp {
            role: SourceRole::FcState,
            integrity: SourceIntegrity::ChecksummedOnly,
            source_id: latest.source_id,
            source_incarnation: latest.source_incarnation,
            source_epoch: latest.source_epoch,
            sequence,
            acquired_at_ns,
            clock: MeasurementClock::HostMonotonic,
        },
        received_at: now,
    });
}

pub(super) fn apply_relative_altitude(
    latest: &mut LinkState,
    time_boot_ms: u32,
    altitude_mm: i32,
    now: Instant,
) {
    if let Some(stamp) = super::measurement::next_relative_altitude_stamp(latest, time_boot_ms, now)
    {
        latest.fc_reports.relative_altitude = Some(RelativeAltitudeReport {
            altitude_m: altitude_mm as f32 / 1_000.0,
            time_boot_ms,
            stamp,
            received_at: now,
        });
    }
}

#[cfg(test)]
mod tests;
