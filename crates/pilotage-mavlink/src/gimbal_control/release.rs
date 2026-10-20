use super::*;
use crate::codec::{GCS_COMPONENT_ID, GCS_SYSTEM_ID};

#[derive(Debug)]
pub(super) struct OwnershipRelease {
    selection: Selection,
    fence: SendFence,
    write: Option<(WriteReceipt, Instant)>,
}

impl ManagerControl {
    pub(super) fn retire_attachment(
        &mut self,
        capabilities: turret_core::PayloadCapabilities,
        epoch: u64,
    ) {
        self.clear_ownership_release();
        self.release_after_stop = None;
        self.last_sent = None;
        // A new attachment cannot discharge the prior device's physical effects.
        // Keep bounded terminal evidence and never feed these effects to its writer.
        if let Ok(Some(effect)) = self.channel.poll_effect() {
            warn!(
                identity = ?effect.command.identity,
                effect = ?effect.effect,
                "detached gimbal effect remains physically unconfirmed"
            );
        }
        let retired = std::mem::replace(
            &mut self.channel,
            PayloadChannel::new(capabilities, epoch, self.started),
        );
        if retired.snapshot_cached().ok().flatten().is_some() {
            self.retired_channel = Some(retired);
        }
        self.connection_epoch = epoch;
    }

    pub(super) fn request_ownership_release(&mut self) {
        if self.requested
            && self.ownership_release.is_none()
            && let Some(selection) = self.selected
        {
            self.ownership_release = Some(OwnershipRelease {
                selection,
                fence: SendFence::default(),
                write: None,
            });
        }
    }

    pub(super) fn queue_stop(&mut self) -> bool {
        if self.release_after_stop.is_some() {
            return true;
        }
        let effect = match self.channel.poll_effect() {
            Ok(Some(effect)) => effect,
            Ok(None) => return self.queue_ownership_release(),
            Err(error) => {
                warn!(%error, "gimbal lifecycle polling failed");
                return false;
            }
        };
        let Some(selection) = self.selected else {
            return false;
        };
        self.request_ownership_release();
        if self.manager_reported_other_owner() {
            return self.channel.resolve_effect(effect.command.identity).is_ok();
        }
        let Ok(stop) = turret_mavlink::stop_command(effect.command.payload) else {
            self.rejection = "gimbal action cancellation is unsupported and remains unconfirmed";
            return false;
        };
        let Ok(demand) = turret_mavlink::encode_payload(stop, selection.device, 0) else {
            return false;
        };
        self.fence.invalidate();
        self.fence = SendFence::default();
        let receipt = WriteReceipt::default();
        let deadline = Instant::now() + Duration::from_millis(250);
        let queued = self.enqueue(demand, None, Some(receipt.clone()), deadline);
        self.release_after_stop = Some((effect, receipt, deadline));
        queued
    }

    fn manager_reported_other_owner(&self) -> bool {
        let Some(selection) = self.selected else {
            return false;
        };
        self.manager_state
            .as_ref()
            .and_then(|state| {
                state
                    .lock()
                    .ok()?
                    .gimbals
                    .managers
                    .get(&(selection.component, selection.device))?
                    .status
            })
            .is_some_and(|(report, received)| {
                received.elapsed() < Duration::from_secs(1)
                    && (report.primary_system, report.primary_component) != (0, 0)
                    && (report.primary_system, report.primary_component)
                        != (GCS_SYSTEM_ID, GCS_COMPONENT_ID)
            })
    }

    pub(super) fn finish_release(&mut self) {
        if let Some((effect, receipt, deadline)) = self.release_after_stop.as_ref() {
            if receipt.sent_at_cached().is_none() && !self.manager_reported_other_owner() {
                if Instant::now() >= *deadline {
                    self.release_after_stop = None;
                    self.queue_stop();
                }
                return;
            }
            if let Err(error) = self.channel.resolve_effect(effect.command.identity) {
                warn!(%error, "gimbal stop responsibility could not be discharged");
                return;
            }
            self.release_after_stop = None;
        }
        self.queue_stop();
    }

    fn clear_ownership_release(&mut self) {
        if let Some(release) = self.ownership_release.take() {
            release.fence.invalidate();
        }
        self.policy.revoke();
        self.requested = false;
    }

    fn queue_ownership_release(&mut self) -> bool {
        let Some(release) = self.ownership_release.as_ref() else {
            return true;
        };
        let Some(selected) = self.selected else {
            return false;
        };
        let same_attachment = selected.component == release.selection.component
            && selected.device == release.selection.device
            && selected.epoch == release.selection.epoch;
        if !same_attachment
            || release
                .write
                .as_ref()
                .is_some_and(|(receipt, _)| receipt.sent_at_cached().is_some())
        {
            self.clear_ownership_release();
            return true;
        }
        let latest = self.manager_state.as_ref().and_then(|state| {
            let state = state.lock().ok()?;
            let report = state
                .gimbals
                .managers
                .get(&(release.selection.component, release.selection.device))?;
            (report.epoch == release.selection.epoch && report.information.is_some())
                .then_some((report.capability_revision, report.status?))
        });
        let Some((revision, (report, received))) = latest else {
            return false;
        };
        let Some(release) = self.ownership_release.as_mut() else {
            return false;
        };
        if release.selection.revision != revision {
            // Ownership belongs to the same attachment even when its axes change.
            // A queued release still has to recheck the current contract at write.
            release.fence.invalidate();
            release.selection.revision = revision;
            release.write = None;
        }
        if let Some((_, deadline)) = release.write.as_ref()
            && Instant::now() < *deadline
        {
            return true;
        }
        let owner = (report.primary_system, report.primary_component);
        if owner != (GCS_SYSTEM_ID, GCS_COMPONENT_ID) && owner != (0, 0) {
            if received.elapsed() < Duration::from_secs(1) {
                self.clear_ownership_release();
            }
            return true;
        }
        self.enqueue_ownership_release()
    }

    fn enqueue_ownership_release(&mut self) -> bool {
        let (Some(state), Some(release)) = (&self.manager_state, self.ownership_release.as_mut())
        else {
            return false;
        };
        release.fence.invalidate();
        release.fence = SendFence::default();
        let selection = release.selection;
        let fence = release.fence.for_manager(
            state.clone(),
            (selection.component, selection.device),
            selection.epoch,
            selection.revision,
            crate::ManagerWriteRequirement::Release,
        );
        let turret_mavlink::Demand::CommandLong { command, params } =
            turret_mavlink::release_command(selection.device)
        else {
            return false;
        };
        let receipt = WriteReceipt::default();
        let deadline = Instant::now() + Duration::from_millis(250);
        let queued = self
            .commands
            .try_send(OutboundCommand {
                command,
                params,
                target_system: self.target_system,
                target_component: selection.component,
                payload_write: None,
                receipt: Some(receipt.clone()),
                send_before: Some(deadline),
                fence: Some(fence),
            })
            .is_ok();
        if queued {
            release.write = Some((receipt, deadline));
        } else {
            self.record_drop();
        }
        queued
    }
}
