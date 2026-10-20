//! Shared transport binding for the portable Turret MAVLink manager client.
use crate::{
    GimbalRateDemand, OutboundCommand, PayloadChannel, PayloadTiming, SendFence, WriteReceipt,
};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc::Sender, watch};
use tracing::warn;
use turret_core::{AngularRate, PayloadCommand, Tick};
use turret_mavlink::ManagerPolicy;

mod discovery;
mod release;
#[cfg(test)]
mod tests;
mod transport;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Selection {
    component: u8,
    device: u8,
    epoch: u64,
    revision: u64,
}

/// Host policy limit for normalized pitch input, radians per second.
pub const MAX_PITCH_RATE_RPS: f32 = 0.8;
/// Host policy limit for normalized yaw input, radians per second.
pub const MAX_YAW_RATE_RPS: f32 = 0.8;
/// MAV_CMD_DO_GIMBAL_MANAGER_CONFIGURE command identifier.
pub const CMD_GIMBAL_CONFIGURE: u16 = 1001;

/// The I/O owner for one concretely selected MAVLink gimbal manager device.
#[derive(Debug)]
pub struct ManagerControl {
    commands: Sender<OutboundCommand>,
    rates: watch::Sender<Option<GimbalRateDemand>>,
    target_system: u8,
    preferred: Option<(u8, u8)>,
    policy: ManagerPolicy,
    channel: PayloadChannel,
    manager_state: Option<std::sync::Arc<std::sync::Mutex<crate::LinkState>>>,
    capabilities: turret_core::PayloadCapabilities,
    started: Instant,
    selected: Option<Selection>,
    manager_time: Option<u32>,
    manager_received: Option<Instant>,
    next_probe: Instant,
    requested: bool,
    claim_receipt: Option<(WriteReceipt, Instant)>,
    last_sent: Option<turret_core::CommandIdentity>,
    timing: Option<PayloadTiming>,
    explicit_validity: bool,
    fence: SendFence,
    release_after_stop: Option<(crate::PayloadEffect, WriteReceipt, Instant)>,
    ownership_release: Option<release::OwnershipRelease>,
    next_id: u64,
    dropped_sends: u64,
    drop_link_loss_stop: bool,
    rejection: &'static str,
}

impl ManagerControl {
    /// Bind portable manager policy and execution to the link's existing lanes.
    pub fn new(
        commands: Sender<OutboundCommand>,
        rates: watch::Sender<Option<GimbalRateDemand>>,
        target_system: u8,
        preferred: Option<(u8, u8)>,
    ) -> Result<Self, turret_core::Error> {
        let pitch = AngularRate::radians_per_second(MAX_PITCH_RATE_RPS)?;
        let yaw = AngularRate::radians_per_second(MAX_YAW_RATE_RPS)?;
        let capabilities = turret_mavlink::manager_capabilities(0, None, pitch, yaw)?;
        let started = Instant::now();
        Ok(Self {
            commands,
            rates,
            target_system,
            preferred,
            policy: ManagerPolicy::new(pitch, yaw)?,
            channel: PayloadChannel::new(capabilities, 1, started),
            manager_state: None,
            capabilities,
            started,
            selected: None,
            manager_time: None,
            manager_received: None,
            next_probe: started,
            requested: false,
            claim_receipt: None,
            last_sent: None,
            timing: None,
            explicit_validity: false,
            fence: SendFence::default(),
            release_after_stop: None,
            ownership_release: None,
            next_id: 0,
            dropped_sends: 0,
            drop_link_loss_stop: false,
            rejection: "gimbal manager ownership has not been confirmed",
        })
    }

    /// Bind receive-side ownership so queued commands revalidate at the socket boundary.
    #[must_use]
    pub fn with_manager_state(
        mut self,
        state: std::sync::Arc<std::sync::Mutex<crate::LinkState>>,
    ) -> Self {
        self.manager_state = Some(state);
        self
    }

    /// Simulation acceptance fault injection; a dropped stop is reported as a failure.
    #[must_use]
    pub fn with_dropped_link_loss_stop(mut self, drop: bool) -> Self {
        self.drop_link_loss_stop = drop;
        self
    }

    /// Current discovered support and implementation limits.
    pub fn capabilities(&self) -> turret_core::PayloadCapabilities {
        self.capabilities
    }

    /// Endpoint clock origin shared by command and observation timestamps.
    pub fn started_at(&self) -> Instant {
        self.started
    }

    /// Read retained command evidence without changing its age.
    pub fn snapshot_cached(&self) -> Option<crate::PayloadSnapshot> {
        self.channel.snapshot_cached().ok().flatten()
    }

    /// Describe the most recent rejected operation.
    pub fn rejection_detail(&self) -> &'static str {
        self.rejection
    }

    /// Convert only unspent host budgets to endpoint-local absolute deadlines.
    pub fn bind_timing(&mut self, timing: Option<pilotage_protocol::AdmittedTiming>) {
        let now = Instant::now();
        self.explicit_validity = timing.is_some_and(|value| value.active_remaining.is_some());
        let send = timing.map_or(Duration::from_millis(250), |value| value.send_remaining);
        let active = timing
            .and_then(|value| value.active_remaining)
            .unwrap_or(Duration::from_millis(300));
        self.timing = now.checked_add(send).map(|send_before| PayloadTiming {
            send_before,
            valid_until: now.checked_add(active),
            ack_before: None,
            completion_before: timing
                .and_then(|value| value.completion_timeout)
                .and_then(|duration| now.checked_add(duration)),
        });
    }

    /// Scale normalized input through Turret policy and enqueue only after manager confirmation.
    pub fn rate_demand(&mut self, pitch: f32, yaw: f32) -> bool {
        if !self.authorize() {
            return false;
        }
        if pitch == 0.0
            && yaw == 0.0
            && self.snapshot_cached().is_some_and(|snapshot| {
                snapshot.command.payload == PayloadCommand::Recenter
                    && snapshot.progress.ended.is_none()
            })
        {
            return true;
        }
        match self
            .policy
            .rate_intent(pitch, yaw, &self.capabilities, self.now())
        {
            Ok(Some(payload)) => self.submit(payload),
            Ok(None) => true,
            Err(_) => {
                self.rejection = "gimbal manager ownership unavailable or rate invalid";
                false
            }
        }
    }

    /// Queue the portable native recenter operation under confirmed ownership.
    pub fn neutral(&mut self) -> bool {
        if !self.authorize() {
            return false;
        }
        match self.policy.recenter_intent(self.now()) {
            Ok(payload) => self.submit(payload),
            Err(_) => false,
        }
    }

    /// Execute a typed gimbal command within the discovered contract and original deadlines.
    pub fn command(&mut self, payload: PayloadCommand) -> bool {
        if matches!(payload, PayloadCommand::Zoom(_)) {
            self.rejection = "camera commands require the independent camera endpoint";
            return false;
        }
        self.authorize() && self.submit(payload)
    }

    /// Fence pending payloads without claiming that previously sent motion stopped.
    pub fn invalidate_pending(&mut self) {
        // The self-release follows all old writes, including an in-progress claim.
        self.fence.invalidate();
        self.request_ownership_release();
        self.revoke_local();
    }

    fn revoke_local(&mut self) {
        self.fence.invalidate();
        self.fence = SendFence::default();
        if let Err(error) = self.channel.revoke() {
            warn!(%error, "gimbal revoke failed");
        }
        self.rates.send(None).ok();
        self.policy.revoke();
        self.requested = false;
        self.claim_receipt = None;
    }

    /// Queue a best-effort stop, then release only this sender's manager ownership.
    pub fn queue_link_loss_stop(&mut self) -> bool {
        self.invalidate_pending();
        if self.drop_link_loss_stop {
            self.rejection = "gimbal stop suppressed by simulation fault injection";
            return false;
        }
        self.queue_stop()
    }

    /// Observe actual write receipts and execute Turret expiry/revocation effects.
    pub fn maintain(&mut self) {
        self.refresh_discovery();
        self.sync_receipts();
        self.queue_stop();
        self.finish_release();
        self.probe_status();
    }

    /// Count local queue refusals separately from actual transport-write evidence.
    #[must_use]
    pub fn dropped_sends(&self) -> u64 {
        self.dropped_sends
    }

    fn tick(&self, at: Instant) -> Option<Tick> {
        at.checked_duration_since(self.started)
            .and_then(|duration| u64::try_from(duration.as_micros()).ok())
            .map(Tick)
    }
    fn now(&self) -> Tick {
        self.tick(Instant::now()).unwrap_or(Tick(u64::MAX))
    }

    fn record_drop(&mut self) {
        self.dropped_sends = self.dropped_sends.wrapping_add(1);
        self.rejection = "gimbal transport queue full or closed";
        warn!(
            dropped = self.dropped_sends,
            "gimbal command queue refused operation"
        );
    }
}
