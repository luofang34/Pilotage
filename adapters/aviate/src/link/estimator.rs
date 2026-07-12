//! Aviate estimator-status authorization for cached numeric groups.

use std::time::Instant;

use pilotage_adapter_api::MeasurementStamp;

use super::LatestAviate;
use super::measurement::next_estimator_status_stamp;

pub(crate) const KNOWN_VALID_FLAGS: u32 = 0x0f;
pub(crate) const QUALITY_GOOD: u32 = 0;
pub(crate) const QUALITY_DEGRADED: u32 = 1;
pub(crate) const QUALITY_UNUSABLE: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EstimatorAuthorization {
    pub(crate) valid_flags: u32,
    pub(crate) quality: u32,
}

impl EstimatorAuthorization {
    fn from_fc(valid_flags: u8, quality: u8) -> Self {
        let valid_flags = u32::from(valid_flags) & KNOWN_VALID_FLAGS;
        match quality {
            2 => Self {
                valid_flags,
                quality: QUALITY_GOOD,
            },
            1 => Self {
                valid_flags,
                quality: QUALITY_DEGRADED,
            },
            0 => Self {
                valid_flags,
                quality: QUALITY_UNUSABLE,
            },
            _ => Self::fail_closed(),
        }
    }

    pub(super) const fn fail_closed() -> Self {
        Self {
            valid_flags: 0,
            quality: QUALITY_UNUSABLE,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct EstimatorStatusUpdate {
    pub(crate) time_usec: u64,
    pub(crate) time_boot_ms: u32,
    pub(crate) authorization: EstimatorAuthorization,
    pub(crate) stamp: MeasurementStamp,
}

pub(super) fn accept_status(
    latest: &mut LatestAviate,
    time_usec: u64,
    valid_flags: u8,
    quality: u8,
    now: Instant,
) {
    let Some(time_boot_ms) = status_time_boot_ms(time_usec) else {
        fail_closed_out_of_range_status(latest);
        return;
    };
    let malformed = !time_usec.is_multiple_of(1_000) || quality > 2;
    if malformed {
        latest.invalid_estimator_statuses = latest.invalid_estimator_statuses.wrapping_add(1);
        if latest
            .estimator_status
            .is_none_or(|current| time_usec >= current.time_usec)
        {
            invalidate_cached_authorization(latest);
        }
    }
    let Some(mut stamp) = next_estimator_status_stamp(latest, time_boot_ms, now) else {
        return;
    };
    stamp.acquired_at_ns = time_usec.saturating_mul(1_000);
    let authorization = if malformed {
        EstimatorAuthorization::fail_closed()
    } else {
        EstimatorAuthorization::from_fc(valid_flags, quality)
    };
    degrade_cached_groups(latest, authorization);
    latest.estimator_status = Some(EstimatorStatusUpdate {
        time_usec,
        time_boot_ms,
        authorization,
        stamp,
    });
}

fn status_time_boot_ms(time_usec: u64) -> Option<u32> {
    u32::try_from(time_usec / 1_000).ok()
}

fn fail_closed_out_of_range_status(latest: &mut LatestAviate) {
    latest.invalid_estimator_statuses = latest.invalid_estimator_statuses.wrapping_add(1);
    invalidate_cached_authorization(latest);
}

pub(super) fn authorization_at(latest: &LatestAviate, time_boot_ms: u32) -> EstimatorAuthorization {
    let time_usec = u64::from(time_boot_ms).saturating_mul(1_000);
    latest
        .estimator_status
        .map_or_else(EstimatorAuthorization::fail_closed, |status| {
            if status.time_usec == time_usec {
                status.authorization
            } else {
                EstimatorAuthorization::fail_closed()
            }
        })
}

fn degrade_cached_groups(latest: &mut LatestAviate, status: EstimatorAuthorization) {
    if let Some(attitude) = latest.attitude.as_mut() {
        attitude.valid_flags &= status.valid_flags;
        attitude.quality = attitude.quality.max(status.quality);
    }
    if let Some(kinematics) = latest.kinematics.as_mut() {
        kinematics.valid_flags &= status.valid_flags;
        kinematics.quality = kinematics.quality.max(status.quality);
    }
}

pub(super) fn invalidate_cached_authorization(latest: &mut LatestAviate) {
    let fail_closed = EstimatorAuthorization::fail_closed();
    if let Some(status) = latest.estimator_status.as_mut() {
        status.authorization = fail_closed;
    }
    degrade_cached_groups(latest, fail_closed);
}

#[cfg(test)]
mod tests;
