use super::*;
use crate::codec::GimbalManagerInformation;

impl ManagerControl {
    pub(super) fn manager_component(&self) -> u8 {
        self.selected_device()
            .or(self.preferred)
            .map_or(0, |key| key.0)
    }

    pub(super) fn information_missing(&self) -> bool {
        self.manager_state
            .as_ref()
            .and_then(|state| {
                let state = state.lock().ok()?;
                let key = self.selected_device()?;
                state.gimbals.managers.get(&key)?.information
            })
            .is_none()
    }

    /// Read discovery for the configured device, or select a single unambiguous manager.
    /// Selection stays fixed for the lifetime of this endpoint.
    pub fn refresh_discovery(&mut self) {
        let report = self.manager_state.as_ref().and_then(|state| {
            let state = state.lock().ok()?;
            let key = self
                .selected
                .map(|s| (s.component, s.device))
                .or(self.preferred);
            let key = match key {
                Some(key) => key,
                None => {
                    let mut candidates = state
                        .gimbals
                        .managers
                        .iter()
                        .filter(|(_, report)| report.information.is_some());
                    let (key, _) = candidates.next()?;
                    if candidates.next().is_some() {
                        return None;
                    }
                    *key
                }
            };
            Some((key, *state.gimbals.managers.get(&key)?))
        });
        let Some((key, report)) = report else {
            return;
        };
        let selection = Selection {
            component: key.0,
            device: key.1,
            epoch: report.epoch,
            revision: report.capability_revision,
        };
        if self.selected != Some(selection) {
            self.bind_discovery(selection, report.information.map(|(info, _)| info));
        }
        if self.selected == Some(selection)
            && let Some((status, received)) = report.status
        {
            self.observe_status(status, received);
        }
    }

    fn bind_discovery(
        &mut self,
        selection: Selection,
        information: Option<GimbalManagerInformation>,
    ) {
        let info = information.map(|information| turret_mavlink::ManagerInformation {
            cap_flags: information.cap_flags,
            roll_min: information.roll_min,
            roll_max: information.roll_max,
            pitch_min: information.pitch_min,
            pitch_max: information.pitch_max,
            yaw_min: information.yaw_min,
            yaw_max: information.yaw_max,
        });
        let result = AngularRate::radians_per_second(MAX_PITCH_RATE_RPS).and_then(|pitch| {
            AngularRate::radians_per_second(MAX_YAW_RATE_RPS).and_then(|yaw| {
                turret_mavlink::manager_capabilities(selection.revision, info, pitch, yaw).or_else(
                    |error| {
                        warn!(%error, "gimbal manager reported invalid limits");
                        turret_mavlink::manager_capabilities(selection.revision, None, pitch, yaw)
                    },
                )
            })
        });
        let attachment_changed = self.selected.is_some_and(|previous| {
            previous.component != selection.component
                || previous.device != selection.device
                || previous.epoch != selection.epoch
        });
        self.invalidate_pending();
        self.manager_time = None;
        self.manager_received = None;
        match result {
            Ok(capabilities) => {
                if attachment_changed {
                    self.retire_attachment(capabilities, selection.epoch);
                } else if let Err(error) = self.channel.update_capabilities(capabilities) {
                    warn!(%error, "gimbal discovery contract update failed");
                    self.selected = None;
                    return;
                }
                self.capabilities = capabilities;
                self.selected = Some(selection);
            }
            Err(error) => {
                warn!(%error, "gimbal manager reported invalid limits");
                self.selected = None;
                self.rejection = "gimbal manager capability report is invalid";
            }
        }
    }

    fn observe_status(&mut self, report: crate::codec::GimbalManagerStatus, received: Instant) {
        // The receive directory orders boot clocks. Comparing receipt identity here
        // prevents repeated cache reads from renewing ownership.
        if self
            .manager_received
            .is_some_and(|previous| received <= previous)
        {
            return;
        }
        let Some(at) = self.tick(received) else {
            return;
        };
        self.manager_time = Some(report.time_boot_ms);
        self.manager_received = Some(received);
        let owned = self.policy.has_authority(at).unwrap_or(false);
        if self
            .policy
            .observe_status(report.primary_system, report.primary_component, at)
            .is_err()
        {
            return;
        }
        if self.claim_receipt.is_some() && !self.policy.claim_due(self.now()).unwrap_or(false) {
            self.claim_receipt = None;
            self.fence.invalidate();
            self.fence = SendFence::default();
        }
        if owned && !self.policy.has_authority(self.now()).unwrap_or(false) {
            self.fence.invalidate();
            self.fence = SendFence::default();
            if let Err(error) = self.channel.revoke() {
                warn!(%error, "gimbal ownership fence failed");
            }
        }
    }

    /// Concrete manager component and device selected for this endpoint.
    pub fn selected_device(&self) -> Option<(u8, u8)> {
        self.selected
            .map(|selection| (selection.component, selection.device))
    }

    /// Read a fresh refusal from this endpoint's manager component.
    pub fn claim_denial_cached(&self, maximum_age: Duration) -> Option<u8> {
        let (component, _) = self.selected_device()?;
        let state = self.manager_state.as_ref()?.lock().ok()?;
        let ack = state.gimbals.configure_acks.get(&component)?;
        (ack.command == CMD_GIMBAL_CONFIGURE
            && ack.result != 0
            && ack.result != 5
            && ack.received_at.elapsed() < maximum_age)
            .then_some(ack.result)
    }

    /// Read only the feedback that belongs to the selected manager and device.
    pub fn device_cached(&self) -> Option<crate::GimbalDeviceAttitude> {
        let selection = self.selected?;
        let (component, device) = (selection.component, selection.device);
        let state = self.manager_state.as_ref()?.lock().ok()?;
        let report = state.gimbals.managers.get(&(component, device))?;
        if report.epoch != selection.epoch || report.capability_revision != selection.revision {
            return None;
        }
        state
            .gimbals
            .device_for_manager_cached(component, device)
            .copied()
    }
}
