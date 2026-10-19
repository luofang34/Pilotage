//! PX4 transport binding for the portable Turret MAVLink manager client.
use pilotage_mavlink::{
    GimbalRateDemand, OutboundCommand, PayloadChannel, PayloadTiming, SendFence, WriteReceipt,
};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc::Sender, watch};
use tracing::warn;
use turret_core::{AngularRate, Effect, PayloadCommand, Tick};
use turret_mavlink::ManagerPolicy;

#[cfg(test)]
mod tests;
mod transport;

pub(crate) const MAX_PITCH_RATE_RPS: f32 = 0.8;
pub(crate) const MAX_YAW_RATE_RPS: f32 = 0.8;
pub(crate) const CMD_GIMBAL_CONFIGURE: u16 = 1001;

/// The I/O owner for one concretely selected MAVLink gimbal manager device.
#[derive(Debug)]
pub struct Px4GimbalControl {
    commands: Sender<OutboundCommand>,
    rates: watch::Sender<Option<GimbalRateDemand>>,
    target_system: u8,
    target_component: u8,
    policy: ManagerPolicy,
    channel: PayloadChannel,
    manager_state: Option<std::sync::Arc<std::sync::Mutex<pilotage_mavlink::LinkState>>>,
    capabilities: turret_core::PayloadCapabilities,
    started: Instant,
    device: Option<u8>,
    manager_time: Option<u32>,
    manager_received: Option<Instant>,
    next_probe: Instant,
    requested: bool,
    claim_receipt: Option<(WriteReceipt, Instant)>,
    last_sent: Option<turret_core::CommandIdentity>,
    timing: Option<PayloadTiming>,
    explicit_validity: bool,
    fence: SendFence,
    release_after_stop: Option<(WriteReceipt, Instant)>,
    next_id: u64,
    dropped_sends: u64,
    drop_link_loss_stop: bool,
    rejection: &'static str,
}

impl Px4GimbalControl {
    /// Bind portable manager policy and execution to the link's existing lanes.
    pub fn new(
        commands: Sender<OutboundCommand>,
        rates: watch::Sender<Option<GimbalRateDemand>>,
        target_system: u8,
        target_component: u8,
    ) -> Result<Self, turret_core::Error> {
        let pitch = AngularRate::radians_per_second(MAX_PITCH_RATE_RPS)?;
        let yaw = AngularRate::radians_per_second(MAX_YAW_RATE_RPS)?;
        let capabilities = turret_mavlink::manager_capabilities(1, pitch, yaw)?;
        let started = Instant::now();
        Ok(Self {
            commands,
            rates,
            target_system,
            target_component,
            policy: ManagerPolicy::new(pitch, yaw)?,
            channel: PayloadChannel::new(capabilities, 1, started),
            manager_state: None,
            capabilities,
            started,
            device: None,
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
        state: std::sync::Arc<std::sync::Mutex<pilotage_mavlink::LinkState>>,
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

    pub(crate) fn capabilities(&self) -> turret_core::PayloadCapabilities {
        self.capabilities
    }

    /// Endpoint clock origin shared by command and observation timestamps.
    pub(crate) fn started_at(&self) -> Instant {
        self.started
    }

    pub(crate) fn snapshot_cached(&self) -> Option<pilotage_mavlink::PayloadSnapshot> {
        self.channel.snapshot_cached().ok().flatten()
    }

    pub(crate) fn rejection_detail(&self) -> &'static str {
        self.rejection
    }

    /// Bind one device from the expected manager; other device reports cannot authorize it.
    pub(crate) fn observe_manager(
        &mut self,
        report: pilotage_mavlink::codec::GimbalManagerStatus,
        received: Instant,
    ) {
        if report.device_id == 0
            || self.device.is_some_and(|device| device != report.device_id)
            || self
                .manager_time
                .is_some_and(|time| report.time_boot_ms <= time)
        {
            return;
        }
        let Some(at) = self.tick(received) else {
            return;
        };
        self.device = Some(report.device_id);
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
        let claim_obsolete =
            self.claim_receipt.is_some() && !self.policy.claim_due(self.now()).unwrap_or(false);
        if claim_obsolete {
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

    /// Convert only unspent host budgets to endpoint-local absolute deadlines.
    pub(crate) fn bind_timing(&mut self, timing: Option<pilotage_protocol::AdmittedTiming>) {
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
        match self.policy.rate_intent(pitch, yaw, self.now()) {
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

    /// Fence pending payloads without claiming that previously sent motion stopped.
    pub(crate) fn invalidate_pending(&mut self) {
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
        self.sync_receipts();
        match self.channel.poll_effect() {
            Ok(Effect::None) => {}
            Ok(_) => {
                self.queue_stop();
            }
            Err(error) => {
                warn!(%error, "gimbal lifecycle polling failed");
            }
        }
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
