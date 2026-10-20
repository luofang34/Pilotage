//! Bounded payload discovery and feedback, isolated from flight-state authorization.

use std::collections::BTreeMap;
use std::time::Instant;

use super::{CommandAckReport, GimbalDeviceAttitude, LinkState};
use crate::codec::{FcMessage, FrameSource, GimbalManagerInformation, GimbalManagerStatus};

mod clock;
use clock::BootClock;

const MAX_GIMBALS: usize = 16;

/// Discovery and ownership for one manager component and managed device.
#[derive(Debug, Clone, Copy)]
pub struct GimbalManagerReport {
    /// Manager-declared operations; absent until discovery or after a clock wrap.
    pub information: Option<(GimbalManagerInformation, Instant)>,
    /// Latest independently advancing ownership report.
    pub status: Option<(GimbalManagerStatus, Instant)>,
    /// Manager boot-clock wrap generation.
    pub epoch: u64,
    /// Changes whenever the declared contract changes or the clock wraps.
    pub capability_revision: u64,
    clock: BootClock,
}

impl Default for GimbalManagerReport {
    fn default() -> Self {
        Self {
            information: None,
            status: None,
            epoch: 1,
            capability_revision: 1,
            clock: BootClock::default(),
        }
    }
}

#[derive(Debug, Default)]
struct DeviceReport {
    clock: BootClock,
    sequence: u32,
    sample: Option<GimbalDeviceAttitude>,
}

/// Payload reports belonging to the selected MAVLink vehicle system.
#[derive(Debug, Default)]
pub struct GimbalReports {
    /// Managers keyed by sender component and declared device ID.
    pub managers: BTreeMap<(u8, u8), GimbalManagerReport>,
    /// Configure acknowledgements keyed by manager component.
    ///
    /// COMMAND_ACK has no device identity. Commands to managers sharing one
    /// component must be serialized; ownership is confirmed by each device's status.
    pub configure_acks: BTreeMap<u8, CommandAckReport>,
    devices: BTreeMap<(u8, u8), DeviceReport>,
}

impl GimbalReports {
    /// Feedback for the device bound by this manager's discovery declaration.
    #[must_use]
    pub fn device_for_manager_cached(
        &self,
        component: u8,
        device: u8,
    ) -> Option<&GimbalDeviceAttitude> {
        self.managers.get(&(component, device))?.information?;
        self.devices
            .get(&device_key(component, device))?
            .sample
            .as_ref()
    }

    fn manager(&mut self, key: (u8, u8)) -> Option<&mut GimbalManagerReport> {
        if key.0 == 0 || key.1 == 0 {
            return None;
        }
        if self.managers.len() >= MAX_GIMBALS && !self.managers.contains_key(&key) {
            return None;
        }
        Some(self.managers.entry(key).or_default())
    }

    fn prepare_manager(&mut self, key: (u8, u8), time: u32) -> Option<&mut GimbalManagerReport> {
        let report = self.manager(key)?;
        let wrapped = report.clock.observe(time)?;
        if wrapped {
            report.information = None;
            report.status = None;
            report.epoch = report.clock.epoch;
            report.capability_revision = report.capability_revision.wrapping_add(1);
            if let Some(device) = self.devices.get_mut(&device_key(key.0, key.1)) {
                device.sample = None;
            }
            self.configure_acks.remove(&key.0);
        }
        self.managers.get_mut(&key)
    }

    fn information(&mut self, component: u8, information: GimbalManagerInformation, now: Instant) {
        let Some(report) =
            self.prepare_manager((component, information.device_id), information.time_boot_ms)
        else {
            return;
        };
        if report.information.is_some_and(|(previous, _)| {
            !clock::advances(information.time_boot_ms, previous.time_boot_ms)
        }) {
            return;
        }
        if report
            .information
            .is_none_or(|(previous, _)| !information.same_contract(previous))
        {
            report.capability_revision = report.capability_revision.wrapping_add(1);
        }
        report.information = Some((information, now));
    }

    fn status(&mut self, component: u8, status: GimbalManagerStatus, now: Instant) {
        let Some(report) = self.prepare_manager((component, status.device_id), status.time_boot_ms)
        else {
            return;
        };
        if report
            .status
            .is_none_or(|(previous, _)| clock::advances(status.time_boot_ms, previous.time_boot_ms))
        {
            report.status = Some((status, now));
        }
    }
}

fn device_key(manager_component: u8, device: u8) -> (u8, u8) {
    if device >= 7 {
        (device, 0)
    } else {
        (manager_component, device)
    }
}

fn addressed_to_gcs(system: u8, component: u8) -> bool {
    (system == 0 || system == crate::codec::GCS_SYSTEM_ID)
        && (component == 0 || component == crate::codec::GCS_COMPONENT_ID)
}

fn apply_feedback(latest: &mut LinkState, component: u8, message: FcMessage, now: Instant) {
    let FcMessage::GimbalDeviceAttitudeStatus {
        time_boot_ms,
        quat_wxyz,
        rates_rps,
        flags,
        failure_flags,
        target_system,
        target_component,
        device_id,
    } = message
    else {
        return;
    };
    let key = (component, device_id);
    let known_route = latest
        .gimbals
        .managers
        .iter()
        .any(|(&(manager, device), report)| {
            report.information.is_some()
                && device_key(manager, device) == key
                && (device_id == 0 || report.clock.accepts_epoch(time_boot_ms))
                && (addressed_to_gcs(target_system, target_component)
                    || (target_system == latest.system_id
                        && (target_component == 0 || target_component == manager)))
        });
    if !known_route || (device_id == 0 && component < 7) || device_id > 6 {
        return;
    }
    if latest.gimbals.devices.len() >= MAX_GIMBALS && !latest.gimbals.devices.contains_key(&key) {
        return;
    }
    let entry = latest.gimbals.devices.entry(key).or_default();
    if !entry.clock.admits_new(time_boot_ms) || entry.clock.observe(time_boot_ms).is_none() {
        return;
    }
    entry.sequence = entry.sequence.wrapping_add(1);
    if failure_flags != 0 {
        tracing::warn!(
            component,
            device_id,
            failure_flags,
            "gimbal device reports a failure"
        );
    }
    entry.sample = Some(GimbalDeviceAttitude {
        source_component: component,
        device_id,
        epoch: entry.clock.epoch,
        sequence: entry.sequence,
        quat_wxyz,
        rates_rps,
        time_boot_ms,
        flags,
        failure_flags,
        received_at: now,
    });
}

fn apply_ack(latest: &mut LinkState, component: u8, message: FcMessage, now: Instant) {
    let FcMessage::CommandAck {
        command,
        result,
        target_system,
        target_component,
    } = message
    else {
        return;
    };
    if !addressed_to_gcs(target_system, target_component)
        || !latest
            .gimbals
            .managers
            .keys()
            .any(|&(manager, _)| manager == component)
    {
        return;
    }
    latest.gimbals.configure_acks.insert(
        component,
        CommandAckReport {
            command,
            result,
            received_at: now,
        },
    );
}

pub(super) fn apply(
    latest: &mut LinkState,
    source: FrameSource,
    message: FcMessage,
    now: Instant,
) -> bool {
    let is_payload = matches!(
        message,
        FcMessage::GimbalManagerInformation(_)
            | FcMessage::GimbalManagerStatus(_)
            | FcMessage::GimbalDeviceAttitudeStatus { .. }
            | FcMessage::CommandAck {
                command: crate::codec::MAV_CMD_DO_GIMBAL_MANAGER_CONFIGURE,
                ..
            }
    );
    if !is_payload {
        return false;
    }
    if source.system_id != latest.system_id || source.component_id == 0 {
        latest.wrong_sources = latest.wrong_sources.wrapping_add(1);
        return true;
    }
    latest.decoded = latest.decoded.wrapping_add(1);
    match message {
        FcMessage::GimbalManagerInformation(report) => {
            latest.gimbals.information(source.component_id, report, now);
        }
        FcMessage::GimbalManagerStatus(report) => {
            latest.gimbals.status(source.component_id, report, now);
        }
        FcMessage::GimbalDeviceAttitudeStatus { .. } => {
            apply_feedback(latest, source.component_id, message, now);
        }
        FcMessage::CommandAck { .. } => apply_ack(latest, source.component_id, message, now),
        _ => {}
    }
    true
}

#[cfg(test)]
mod tests;
