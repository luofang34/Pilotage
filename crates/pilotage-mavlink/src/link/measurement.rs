//! Measurement ordering and simulator reset quarantine.

use std::time::{Duration, Instant};

use pilotage_adapter_api::{MeasurementClock, MeasurementStamp, SourceIntegrity, SourceRole};
use tracing::warn;

use super::{LinkState, ResetPolicy};

const RESET_CANDIDATE_MAX_MS: u32 = 5_000;
pub(super) const RESET_SILENCE: Duration = Duration::from_secs(3);
pub(super) const RESET_RECEIVE_DWELL: Duration = Duration::from_millis(250);
pub(super) const RESET_SOURCE_DWELL_MS: u32 = 250;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MeasurementGroup {
    Attitude,
    Kinematics,
    EstimatorStatus,
}

/// A quarantined low-boot-clock candidate awaiting simulator-only
/// reset confirmation (source progress + silence + receive dwell).
#[derive(Debug, Clone, Copy)]
pub struct ResetCandidate {
    group: MeasurementGroup,
    first_time_ms: u32,
    latest_time_ms: u32,
    started_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimeObservation {
    CurrentEpoch,
    PendingReset,
    NewEpoch,
    Rejected,
}

pub(super) fn serial_is_newer(candidate: u32, current: u32) -> bool {
    let distance = candidate.wrapping_sub(current);
    distance != 0 && distance < (1_u32 << 31)
}

fn begin_source_epoch(latest: &mut LinkState, time_boot_ms: u32) {
    latest.source_epoch = latest.source_epoch.wrapping_add(1);
    latest.last_source_time_ms = Some(time_boot_ms);
    latest.last_accepted_at = None;
    latest.pending_reset = None;
    latest.attitude = None;
    latest.kinematics = None;
    latest.estimator_status = None;
    latest.source_resets = latest.source_resets.wrapping_add(1);
    warn!(
        source_epoch = latest.source_epoch,
        time_boot_ms, "MAVLink acquisition clock entered a new epoch"
    );
}

fn accepted_stream_is_silent(latest: &LinkState, now: Instant) -> bool {
    latest.last_accepted_at.is_none_or(|accepted| {
        now.checked_duration_since(accepted)
            .unwrap_or(Duration::ZERO)
            >= RESET_SILENCE
    })
}

fn reject_reordered(latest: &mut LinkState) -> TimeObservation {
    latest.reordered_measurements = latest.reordered_measurements.wrapping_add(1);
    TimeObservation::Rejected
}

fn reset_candidate_or_reject(
    latest: &mut LinkState,
    group: MeasurementGroup,
    time_boot_ms: u32,
    now: Instant,
) -> TimeObservation {
    if time_boot_ms > RESET_CANDIDATE_MAX_MS
        || latest.reset_policy == ResetPolicy::Conservative
        || !accepted_stream_is_silent(latest, now)
    {
        reject_reordered(latest)
    } else {
        advance_reset_candidate(latest, group, time_boot_ms, now)
    }
}

fn advance_reset_candidate(
    latest: &mut LinkState,
    group: MeasurementGroup,
    time_boot_ms: u32,
    now: Instant,
) -> TimeObservation {
    let Some(mut candidate) = latest.pending_reset else {
        latest.pending_reset = Some(ResetCandidate {
            group,
            first_time_ms: time_boot_ms,
            latest_time_ms: time_boot_ms,
            started_at: now,
        });
        latest.suspected_resets = latest.suspected_resets.wrapping_add(1);
        return TimeObservation::PendingReset;
    };
    if candidate.group != group || !serial_is_newer(time_boot_ms, candidate.latest_time_ms) {
        return reject_reordered(latest);
    }
    candidate.latest_time_ms = time_boot_ms;
    latest.pending_reset = Some(candidate);
    let source_dwell = time_boot_ms.wrapping_sub(candidate.first_time_ms);
    let receive_dwell = now
        .checked_duration_since(candidate.started_at)
        .unwrap_or(Duration::ZERO);
    if source_dwell < RESET_SOURCE_DWELL_MS || receive_dwell < RESET_RECEIVE_DWELL {
        return TimeObservation::PendingReset;
    }
    begin_source_epoch(latest, time_boot_ms);
    TimeObservation::NewEpoch
}

fn observe_source_time(
    latest: &mut LinkState,
    group: MeasurementGroup,
    current_time_ms: Option<u32>,
    time_boot_ms: u32,
    now: Instant,
) -> TimeObservation {
    if let Some(high_water) = latest.last_source_time_ms
        && time_boot_ms < high_water
        && serial_is_newer(time_boot_ms, high_water)
    {
        begin_source_epoch(latest, time_boot_ms);
        return TimeObservation::NewEpoch;
    }
    if let Some(high_water) = latest.last_source_time_ms
        && time_boot_ms < high_water
        && high_water.wrapping_sub(time_boot_ms) > latest.maximum_inter_group_skew_ms
    {
        return reset_candidate_or_reject(latest, group, time_boot_ms, now);
    }
    let Some(current) = current_time_ms else {
        latest.pending_reset = None;
        return observe_initial_group(latest, time_boot_ms);
    };
    if time_boot_ms == current {
        return TimeObservation::CurrentEpoch;
    }
    if time_boot_ms > current && serial_is_newer(time_boot_ms, current) {
        latest.pending_reset = None;
        return TimeObservation::CurrentEpoch;
    }
    reset_candidate_or_reject(latest, group, time_boot_ms, now)
}

fn observe_initial_group(latest: &mut LinkState, time_boot_ms: u32) -> TimeObservation {
    let Some(high_water) = latest.last_source_time_ms else {
        return TimeObservation::CurrentEpoch;
    };
    if time_boot_ms == high_water
        || (time_boot_ms > high_water && serial_is_newer(time_boot_ms, high_water))
    {
        return TimeObservation::CurrentEpoch;
    }
    if time_boot_ms < high_water
        && high_water.wrapping_sub(time_boot_ms) <= latest.maximum_inter_group_skew_ms
    {
        return TimeObservation::CurrentEpoch;
    }
    reject_reordered(latest)
}

pub(super) fn next_attitude_stamp(
    latest: &mut LinkState,
    time_boot_ms: u32,
    now: Instant,
) -> Option<MeasurementStamp> {
    let current = latest
        .attitude
        .map(|update| (update.time_boot_ms, update.stamp));
    next_group_stamp(
        current,
        latest,
        MeasurementGroup::Attitude,
        time_boot_ms,
        now,
    )
}

pub(super) fn next_kinematics_stamp(
    latest: &mut LinkState,
    time_boot_ms: u32,
    now: Instant,
) -> Option<MeasurementStamp> {
    let current = latest
        .kinematics
        .map(|update| (update.time_boot_ms, update.stamp));
    next_group_stamp(
        current,
        latest,
        MeasurementGroup::Kinematics,
        time_boot_ms,
        now,
    )
}

pub(super) fn next_estimator_status_stamp(
    latest: &mut LinkState,
    time_boot_ms: u32,
    now: Instant,
) -> Option<MeasurementStamp> {
    let current = latest
        .estimator_status
        .map(|update| (update.time_boot_ms, update.stamp));
    next_group_stamp(
        current,
        latest,
        MeasurementGroup::EstimatorStatus,
        time_boot_ms,
        now,
    )
}

fn next_group_stamp(
    current: Option<(u32, MeasurementStamp)>,
    latest: &mut LinkState,
    group: MeasurementGroup,
    time_boot_ms: u32,
    now: Instant,
) -> Option<MeasurementStamp> {
    let observation = observe_source_time(
        latest,
        group,
        current.map(|(time, _)| time),
        time_boot_ms,
        now,
    );
    let current = match observation {
        TimeObservation::CurrentEpoch => current,
        TimeObservation::NewEpoch => None,
        TimeObservation::PendingReset | TimeObservation::Rejected => return None,
    };
    let sequence = match current {
        None => 0,
        Some((current_time, _)) if current_time == time_boot_ms => {
            latest.duplicate_measurements = latest.duplicate_measurements.wrapping_add(1);
            return None;
        }
        Some((current_time, stamp)) if serial_is_newer(time_boot_ms, current_time) => {
            stamp.sequence.wrapping_add(1)
        }
        // observe_source_time admits only equal or serially newer times
        // for a group that already has a measurement, so this arm is the
        // exhaustiveness fail-safe: an older time that ever slipped
        // through would still reject rather than regress the sequence.
        Some(_) => return reject_group(latest),
    };
    update_high_water(latest, time_boot_ms);
    latest.last_accepted_at = Some(now);
    Some(MeasurementStamp {
        role: SourceRole::OperationalEstimate,
        // MAVLink frames are CRC-checked but unsigned.
        integrity: SourceIntegrity::ChecksummedOnly,
        source_id: latest.source_id,
        source_incarnation: latest.source_incarnation,
        source_epoch: latest.source_epoch,
        sequence,
        acquired_at_ns: u64::from(time_boot_ms).wrapping_mul(1_000_000),
        clock: MeasurementClock::VehicleBoot,
    })
}

fn update_high_water(latest: &mut LinkState, time_boot_ms: u32) {
    if latest
        .last_source_time_ms
        .is_none_or(|current| serial_is_newer(time_boot_ms, current))
    {
        latest.last_source_time_ms = Some(time_boot_ms);
    }
}

fn reject_group(latest: &mut LinkState) -> Option<MeasurementStamp> {
    latest.reordered_measurements = latest.reordered_measurements.wrapping_add(1);
    None
}
