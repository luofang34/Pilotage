use super::*;
use pilotage_mavlink::codec::{GCS_COMPONENT_ID, GCS_SYSTEM_ID};
use turret_mavlink::Demand;

impl Px4GimbalControl {
    pub(super) fn authorize(&mut self) -> bool {
        self.sync_receipts();
        if self.release_after_stop.is_some() {
            self.rejection = "gimbal stop write is pending";
            return false;
        }
        if !self.requested {
            self.fence.invalidate();
            self.fence = SendFence::default();
            if self
                .policy
                .request_claim(GCS_SYSTEM_ID, GCS_COMPONENT_ID)
                .is_err()
            {
                return false;
            }
            self.requested = true;
        }
        self.probe_status();
        let Some(device) = self.device else {
            self.rejection = "gimbal manager device is not discovered";
            return false;
        };
        if self.policy.has_authority(self.now()).unwrap_or(false) {
            return true;
        }
        if self.policy.claim_due(self.now()).unwrap_or(false) && self.claim_receipt.is_none() {
            let receipt = WriteReceipt::default();
            let until = Instant::now() + Duration::from_millis(250);
            if self.enqueue(
                turret_mavlink::claim_command(GCS_SYSTEM_ID, GCS_COMPONENT_ID, device),
                None,
                Some(receipt.clone()),
                until,
            ) {
                self.claim_receipt = Some((receipt, until));
            }
        }
        self.rejection = "gimbal manager has not confirmed this client as primary";
        false
    }

    pub(super) fn probe_status(&mut self) {
        let now = Instant::now();
        if now < self.next_probe {
            return;
        }
        self.next_probe = now + Duration::from_millis(400);
        self.enqueue(
            turret_mavlink::request_manager_status(),
            None,
            None,
            now + Duration::from_millis(250),
        );
    }

    pub(super) fn sync_receipts(&mut self) {
        if let Some((receipt, deadline)) = self.claim_receipt.as_ref() {
            if let Some(sent) = receipt.sent_at_cached().and_then(|sent| self.tick(sent)) {
                self.policy.record_claim_sent(sent).ok();
                self.claim_receipt = None;
            } else if Instant::now() >= *deadline {
                self.claim_receipt = None;
            }
        }
        if let Some(snapshot) = self.snapshot_cached()
            && self.last_sent != Some(snapshot.command.identity)
            && let Some(sent) = snapshot.progress.sent_at
        {
            if let Err(error) = self.policy.record_sent(snapshot.command.payload, sent) {
                warn!(%error, "gimbal policy rejected write receipt");
            }
            self.last_sent = Some(snapshot.command.identity);
        }
    }

    pub(super) fn submit(&mut self, payload: PayloadCommand) -> bool {
        let Some(device) = self.device else {
            return false;
        };
        if self.timing.is_none() {
            self.bind_timing(None);
        }
        let Some(mut timing) = self.timing else {
            return false;
        };
        let Some(status_expires) = self
            .manager_received
            .and_then(|at| at.checked_add(Duration::from_secs(1)))
        else {
            return false;
        };
        timing.send_before = timing.send_before.min(status_expires);
        if payload.is_continuous() {
            timing.valid_until = timing.valid_until.map(|until| until.min(status_expires));
            timing.completion_before = None;
            if let Some(valid) = timing.valid_until {
                timing.send_before = timing.send_before.min(valid);
            }
        } else {
            if !self.explicit_validity {
                timing.valid_until = None;
            }
            if let Some(valid) = timing.valid_until {
                timing.send_before = timing.send_before.min(valid);
            }
            if let Some(complete) = timing.completion_before {
                timing.send_before = timing.send_before.min(complete);
            }
        }
        let Ok(demand) = turret_mavlink::encode_payload(payload, device, 0) else {
            return false;
        };
        self.next_id = self.next_id.wrapping_add(1);
        let write = match self.channel.submit(payload, self.next_id, timing) {
            Ok(write) => write,
            Err(error) => {
                warn!(%error, "gimbal executor refused operation");
                self.rejection = "gimbal command expired or violates the payload contract";
                return false;
            }
        };
        self.enqueue(demand, Some(write), None, timing.send_before)
    }

    pub(super) fn enqueue(
        &mut self,
        demand: Demand,
        payload_write: Option<pilotage_mavlink::PayloadWrite>,
        receipt: Option<WriteReceipt>,
        send_before: Instant,
    ) -> bool {
        let fence = self.command_fence(demand, payload_write.is_some());
        let sent = match demand {
            Demand::CommandLong { command, params } => self
                .commands
                .try_send(OutboundCommand {
                    command,
                    params,
                    target_system: self.target_system,
                    target_component: self.target_component,
                    send_before: Some(send_before),
                    fence: Some(fence),
                    payload_write,
                    receipt,
                })
                .is_ok(),
            Demand::AttitudeRate {
                flags,
                gimbal_device_id,
                pitch_rps,
                yaw_rps,
                ..
            } if flags == turret_mavlink::HORIZON_VEHICLE_YAW_FLAGS => self
                .rates
                .send(Some(GimbalRateDemand {
                    pitch_rps,
                    yaw_rps,
                    gimbal_device_id,
                    target_system: self.target_system,
                    target_component: self.target_component,
                    send_before: Some(send_before),
                    fence: Some(fence),
                    payload_write,
                    receipt,
                }))
                .is_ok(),
            _ => false,
        };
        if !sent {
            self.record_drop();
        }
        sent
    }

    fn command_fence(&self, demand: Demand, payload: bool) -> SendFence {
        use pilotage_mavlink::ManagerWriteRequirement as Requirement;
        let requirement = match demand {
            Demand::CommandLong {
                command: 1001,
                params,
            } if params[0] >= 0.0 => Some(Requirement::Unclaimed),
            Demand::CommandLong { command: 1000, .. } => Some(Requirement::Primary),
            Demand::AttitudeRate { .. } => Some(if payload {
                Requirement::Primary
            } else {
                Requirement::Stop
            }),
            _ => None,
        };
        if let Some(requirement) = requirement {
            if let (Some(state), Some(device)) = (&self.manager_state, self.device) {
                return self.fence.for_manager(state.clone(), device, requirement);
            }
            let invalid = SendFence::default();
            invalid.invalidate();
            return invalid;
        }
        self.fence.clone()
    }

    pub(super) fn queue_stop(&mut self) -> bool {
        let Some(device) = self.device else {
            return true;
        };
        self.fence.invalidate();
        self.fence = SendFence::default();
        let Ok(stop) = turret_mavlink::stop_command(PayloadCommand::Recenter) else {
            return false;
        };
        let Ok(demand) = turret_mavlink::encode_payload(stop, device, 0) else {
            return false;
        };
        let receipt = WriteReceipt::default();
        let deadline = Instant::now() + Duration::from_millis(250);
        let queued = self.enqueue(demand, None, Some(receipt.clone()), deadline);
        self.release_after_stop = Some((receipt, deadline));
        queued
    }

    fn manager_reported_other_owner(&self) -> bool {
        self.manager_state
            .as_ref()
            .and_then(|state| state.lock().ok()?.fc_reports.gimbal_manager)
            .is_some_and(|(report, received)| {
                Some(report.device_id) == self.device
                    && received.elapsed() < Duration::from_secs(1)
                    && (report.primary_system, report.primary_component)
                        != (GCS_SYSTEM_ID, GCS_COMPONENT_ID)
            })
    }

    pub(super) fn finish_release(&mut self) {
        let Some((receipt, deadline)) = self.release_after_stop.as_ref() else {
            return;
        };
        if receipt.sent_at_cached().is_none() && !self.manager_reported_other_owner() {
            if Instant::now() >= *deadline {
                self.queue_stop();
            }
            return;
        }
        self.release_after_stop = None;
        if let Some(device) = self.device {
            self.enqueue(
                turret_mavlink::release_command(device),
                None,
                None,
                Instant::now() + Duration::from_millis(250),
            );
        }
        self.policy.revoke();
        self.requested = false;
    }
}
