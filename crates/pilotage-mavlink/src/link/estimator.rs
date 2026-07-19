//! Aviate estimator-status authorization for cached numeric groups.

use std::time::Instant;

use pilotage_adapter_api::MeasurementStamp;

use super::LinkState;
use super::measurement::next_estimator_status_stamp;

/// The validity bits this dialect defines; unknown bits are masked off.
pub const KNOWN_VALID_FLAGS: u32 = 0x0f;
/// Canonical quality: fully usable.
pub const QUALITY_GOOD: u32 = 0;
/// Canonical quality: degraded but present.
pub const QUALITY_DEGRADED: u32 = 1;
/// Canonical quality: unusable; consumers must not act on the values.
pub const QUALITY_UNUSABLE: u32 = 2;

/// The authorization an estimator-status report grants to cached
/// numeric groups: masked validity bits plus a canonical quality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EstimatorAuthorization {
    /// Validity bits, masked to [`KNOWN_VALID_FLAGS`].
    pub valid_flags: u32,
    /// Canonical quality (`QUALITY_GOOD` / `_DEGRADED` / `_UNUSABLE`).
    pub quality: u32,
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

/// One accepted estimator-status report with its acquisition stamp.
#[derive(Debug, Clone, Copy)]
pub struct EstimatorStatusUpdate {
    /// Source-reported time, microseconds.
    pub time_usec: u64,
    /// Milliseconds since FC boot derived from `time_usec`.
    pub time_boot_ms: u32,
    /// The authorization this report grants.
    pub authorization: EstimatorAuthorization,
    /// Identity and acquisition stamp for this report.
    pub stamp: MeasurementStamp,
}

pub(super) fn accept_status(
    latest: &mut LinkState,
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

fn fail_closed_out_of_range_status(latest: &mut LinkState) {
    latest.invalid_estimator_statuses = latest.invalid_estimator_statuses.wrapping_add(1);
    invalidate_cached_authorization(latest);
}

/// Longest a standard-status authorization stays current for later
/// numeric groups. PX4 streams ESTIMATOR_STATUS at its own (slower)
/// rate; beyond this lag the numeric group flies unauthorized and
/// fails closed.
const MAX_STANDARD_STATUS_LAG_MS: u32 = 2_000;

pub(super) fn authorization_at(latest: &LinkState, time_boot_ms: u32) -> EstimatorAuthorization {
    let Some(status) = latest.estimator_status else {
        return EstimatorAuthorization::fail_closed();
    };
    match latest.authorization_source {
        // Aviate emits a status for every numeric millisecond:
        // authorization requires the exact pair.
        super::AuthorizationSource::AviatePrivate => {
            let time_usec = u64::from(time_boot_ms).saturating_mul(1_000);
            if status.time_usec == time_usec {
                status.authorization
            } else {
                EstimatorAuthorization::fail_closed()
            }
        }
        // Standard-status dialects authorize from the most recent
        // report within a bounded lag. The wrapping distance keeps the
        // comparison correct across a boot-clock wrap: a status AHEAD
        // of the numeric (distance in the upper half) is also current.
        super::AuthorizationSource::StandardEstimatorStatus => {
            let distance = time_boot_ms.wrapping_sub(status.time_boot_ms);
            if distance <= MAX_STANDARD_STATUS_LAG_MS || distance > u32::MAX / 2 {
                status.authorization
            } else {
                EstimatorAuthorization::fail_closed()
            }
        }
    }
}

/// Maps standard ESTIMATOR_STATUS (msg 230) flags onto this crate's
/// wire authorization vocabulary: bit 0 (attitude) grants attitude and
/// rates, horizontal+vertical position bits grant position, and
/// horizontal+vertical velocity bits grant velocity. Wire quality is
/// GOOD (2) with attitude, position, and velocity all present,
/// DEGRADED (1) with attitude alone, otherwise UNUSABLE (0).
pub(super) fn standard_authorization(flags: u16) -> (u8, u8) {
    const ATTITUDE: u16 = 1;
    const VELOCITY_HORIZ: u16 = 2;
    const VELOCITY_VERT: u16 = 4;
    const POS_HORIZ_REL: u16 = 8;
    const POS_HORIZ_ABS: u16 = 16;
    const POS_VERT_ABS: u16 = 32;
    let attitude = flags & ATTITUDE != 0;
    let position = flags & (POS_HORIZ_REL | POS_HORIZ_ABS) != 0 && flags & POS_VERT_ABS != 0;
    let velocity = flags & VELOCITY_HORIZ != 0 && flags & VELOCITY_VERT != 0;
    let mut valid: u8 = 0;
    if attitude {
        valid |= 0b0011;
    }
    if position {
        valid |= 0b0100;
    }
    if velocity {
        valid |= 0b1000;
    }
    let quality = if attitude && position && velocity {
        2
    } else if attitude {
        1
    } else {
        0
    };
    (valid, quality)
}

fn degrade_cached_groups(latest: &mut LinkState, status: EstimatorAuthorization) {
    if let Some(attitude) = latest.attitude.as_mut() {
        attitude.valid_flags &= status.valid_flags;
        attitude.quality = attitude.quality.max(status.quality);
    }
    if let Some(kinematics) = latest.kinematics.as_mut() {
        kinematics.valid_flags &= status.valid_flags;
        kinematics.quality = kinematics.quality.max(status.quality);
    }
}

pub(super) fn invalidate_cached_authorization(latest: &mut LinkState) {
    let fail_closed = EstimatorAuthorization::fail_closed();
    if let Some(status) = latest.estimator_status.as_mut() {
        status.authorization = fail_closed;
    }
    degrade_cached_groups(latest, fail_closed);
}

#[cfg(test)]
mod tests;
