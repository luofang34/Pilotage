//! Camera reports cannot authorize vehicle estimates.
use super::LinkState;
use crate::codec::CameraMessage;
use std::time::Instant;

/// Independently discovered camera and its latest lens setting.
#[derive(Debug, Clone, Copy)]
pub struct CameraReport {
    /// True only while new discovery reports continue to arrive.
    pub available: bool,
    /// Revision of the declared operations and feedback support.
    pub capability_revision: u64,
    /// CAMERA_CAP_FLAGS from CAMERA_INFORMATION; absent until discovery.
    pub flags: Option<u32>,
    /// Associated gimbal, when the camera reports one.
    pub gimbal_id: u8,
    /// Last camera device boot timestamp admitted.
    pub time_boot_ms: u32,
    /// Reboot/wrap generation; timestamps are ordered within this epoch.
    pub epoch: u64,
    /// Actual lens travel fraction, never a commanded target.
    pub travel_fraction: Option<f32>,
    /// Timestamp of the last CAMERA_SETTINGS report.
    pub settings_time_boot_ms: Option<u32>,
    /// Local receive instant of the latest information report.
    pub information_received_at: Option<Instant>,
    /// Device timestamp of the latest information report, for duplicate rejection.
    pub information_time_boot_ms: Option<u32>,
    /// Local receive instant of the latest settings report.
    pub settings_received_at: Option<Instant>,
}

pub(super) fn apply(latest: &mut LinkState, component: u8, message: CameraMessage, now: Instant) {
    let (time, device) = match message {
        CameraMessage::Information {
            time_boot_ms,
            device_id,
            ..
        }
        | CameraMessage::Settings {
            time_boot_ms,
            device_id,
            ..
        } => (time_boot_ms, device_id),
    };
    if device > 6 || (device == 0 && component < 7) {
        return;
    }
    let entry = latest
        .fc_reports
        .cameras
        .entry((component, device))
        .or_insert(CameraReport {
            available: false,
            information_time_boot_ms: None,
            capability_revision: 1,
            flags: None,
            gimbal_id: 0,
            time_boot_ms: time,
            epoch: 1,
            travel_fraction: None,
            settings_time_boot_ms: None,
            information_received_at: None,
            settings_received_at: None,
        });
    // A late packet cannot establish a reboot; reconnect establishes a new incarnation.
    if time < entry.time_boot_ms {
        return;
    }
    entry.time_boot_ms = time;
    match message {
        CameraMessage::Information {
            flags, gimbal_id, ..
        } => {
            if entry
                .information_time_boot_ms
                .is_some_and(|previous| time <= previous)
            {
                return;
            }
            if entry.flags != Some(flags) || !entry.available {
                entry.capability_revision = entry.capability_revision.wrapping_add(1);
            }
            entry.available = true;
            entry.flags = Some(flags);
            entry.gimbal_id = gimbal_id;
            entry.information_received_at = Some(now);
            entry.information_time_boot_ms = Some(time);
        }
        CameraMessage::Settings {
            travel_fraction, ..
        } => {
            if entry
                .settings_time_boot_ms
                .is_some_and(|previous| time <= previous)
            {
                return;
            }
            if entry.travel_fraction.is_some() != travel_fraction.is_some() {
                entry.capability_revision = entry.capability_revision.wrapping_add(1);
            }
            entry.travel_fraction = travel_fraction;
            entry.settings_time_boot_ms = Some(time);
            entry.settings_received_at = Some(now);
        }
    }
}

impl LinkState {
    /// Expire discovery without deleting known operations or resetting their identities.
    pub fn expire_camera_reports(&mut self, now: Instant) {
        for report in self.fc_reports.cameras.values_mut() {
            if report.available
                && report.information_received_at.is_none_or(|received| {
                    now.saturating_duration_since(received) >= std::time::Duration::from_secs(6)
                })
            {
                report.available = false;
                report.capability_revision = report.capability_revision.wrapping_add(1);
            }
        }
    }
}

#[cfg(test)]
mod tests;
