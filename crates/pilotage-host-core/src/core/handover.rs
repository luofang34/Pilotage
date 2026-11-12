//! A plan loaded while another one flies. The new plan's principal opens
//! its own session and asks the flying principal for the motion scope by a
//! transfer; the flying principal frames until the transfer commits, and
//! the new one frames from its next tick on. The flight controller, which
//! descends after half a second without a setpoint, is never left
//! uncommanded. A new principal that does not take over in time is dropped,
//! and the flown plan goes on.

use pilotage_session::ClientKey;
use tracing::{info, warn};

use super::HostCore;
use crate::automation::{MISSION_CLIENT, MissionTask};
use crate::output::HostEvent;

/// The client key of a principal that takes over from one under
/// [`MISSION_CLIENT`]; the next plan after it takes [`MISSION_CLIENT`]
/// again. Two principals are never registered under one key.
const SUCCESSOR_CLIENT: ClientKey = ClientKey::new(u64::MAX - 3);

/// How long a new principal may take to hold the motion scope before the
/// core gives it up, nanoseconds. In the core the whole exchange takes
/// one call; the bound covers a holder that never offers the scope.
const TAKEOVER_TIMEOUT_NS: u64 = 2_000_000_000;

/// A principal over a plan loaded while another one flies.
pub(super) struct Incoming {
    pub(super) task: Box<MissionTask>,
    /// When it was loaded, nanoseconds since the core's origin.
    since_ns: u64,
}

impl HostCore {
    /// Starts `principal` over a newly loaded plan. With no principal that
    /// holds the motion scope, it replaces the present one at once; else
    /// it takes the scope over by a transfer.
    pub(super) fn begin_principal(&mut self, mut principal: Box<MissionTask>) {
        let now_ns = self.last_now.as_nanos();
        if let Some(stale) = self.incoming.take() {
            self.retire(stale.task);
        }
        let flying = self
            .principal
            .as_ref()
            .filter(|flying| flying.status().lease_generation.is_some());
        let client = match flying.map(|flying| flying.client()) {
            Some(MISSION_CLIENT) => SUCCESSOR_CLIENT,
            _ => MISSION_CLIENT,
        };
        principal.bind_client(client);
        principal.set_now(now_ns);
        if flying.is_some() {
            principal.take_over();
            principal.start();
            self.incoming = Some(Incoming {
                task: principal,
                since_ns: now_ns,
            });
        } else {
            if let Some(previous) = self.principal.take() {
                self.retire(previous);
            }
            principal.start();
            self.principal = Some(principal);
            self.next_mission_tick = now_ns;
        }
        self.exchange();
    }

    /// Ends a handover that is done: the new principal holds the motion
    /// scope and replaces the flying one, or it failed or ran out of time
    /// and is dropped.
    pub(super) fn settle_handover(&mut self) {
        let Some(incoming) = self.incoming.as_ref() else {
            return;
        };
        let status = incoming.task.status();
        let refused = if status.fenced || status.closed {
            Some("the new plan's principal ended before it took over")
        } else if self.last_now.as_nanos().saturating_sub(incoming.since_ns) > TAKEOVER_TIMEOUT_NS {
            Some("the flying plan did not hand the motion scope over in time")
        } else {
            None
        };
        if status.lease_generation.is_some() {
            let Some(incoming) = self.incoming.take() else {
                return;
            };
            info!("the new plan holds the motion scope and flies");
            if let Some(previous) = self.principal.replace(incoming.task) {
                self.retire(previous);
            }
            self.counters_logged = false;
            self.events.push(HostEvent::NewPlanFlying);
        } else if let Some(reason) = refused {
            warn!(
                reason,
                "the new plan did not take over; the flown plan goes on"
            );
            if let Some(incoming) = self.incoming.take() {
                self.retire(incoming.task);
            }
            self.events.push(HostEvent::NewPlanRefused { reason });
        }
    }

    /// Ends a principal the core no longer steps: its counters go to the
    /// log and its client's queues are forgotten. It does not clear the
    /// guidance group, which the principal that flies now owns.
    fn retire(&mut self, principal: Box<MissionTask>) {
        principal.log_counters();
        self.actor.disconnect(principal.client());
    }
}
