//! Host-issued control windows bounded by the client's original probe time.

use std::collections::VecDeque;

use pilotage_protocol::wire;

use super::ClientEngine;
use crate::ClientAction;

const PROBE_INTERVAL_NS: u64 = 100_000_000;
const NS_PER_MS: u64 = 1_000_000;

#[derive(Debug, Default)]
pub(super) struct ControlWindow {
    windows: VecDeque<ReceivedWindow>,
    probes: VecDeque<u64>,
    last_probe_at: Option<u64>,
}

#[derive(Debug)]
struct ReceivedWindow {
    id: u64,
    received_at_ns: u64,
    expires_at_ns: u64,
}

impl ControlWindow {
    pub(super) fn id_at(&self, now_ns: u64) -> u64 {
        self.windows
            .iter()
            .find(|window| now_ns >= window.received_at_ns && now_ns < window.expires_at_ns)
            .map_or(0, |window| window.id)
    }

    pub(super) fn receive(&mut self, pong: &wire::Pong, now_ns: u64) {
        let Some(sent) = pong.echoed_sender_sent_at.as_ref().map(|stamp| stamp.nanos) else {
            return;
        };
        let latest = self.windows.front().map_or(0, |window| window.id);
        let advance = pong.control_window_id.wrapping_sub(latest);
        let expires = sent.saturating_add(u64::from(pong.control_window_ms) * 1_000_000);
        if pong.nonce != sent
            || !self.probes.contains(&sent)
            || pong.control_window_id == 0
            || advance == 0
            || advance >= (1_u64 << 63)
            || now_ns < sent
            || now_ns >= expires
        {
            return;
        }
        self.probes.retain(|probe| *probe != sent);
        // The event time is a millisecond bucket, not a precise receive time.
        // Only a later bucket proves that an input followed this reply.
        self.windows.push_front(ReceivedWindow {
            id: pong.control_window_id,
            received_at_ns: now_ns.saturating_add(NS_PER_MS),
            // Receipt cannot renew a window delayed in either direction.
            expires_at_ns: expires,
        });
        // Coalesced replies must not discard an already valid input's window.
        if self.windows.len() > 4 {
            self.windows.pop_back();
        }
    }

    fn probe(&mut self, now_ns: u64) -> Option<ClientAction> {
        if self
            .last_probe_at
            .is_some_and(|previous| now_ns.saturating_sub(previous) < PROBE_INTERVAL_NS)
        {
            return None;
        }
        self.last_probe_at = Some(now_ns);
        // Transport events use whole milliseconds. Round the wire stamp down
        // so a same-millisecond reply cannot look earlier than its own probe.
        let wire_at_ns = now_ns / NS_PER_MS * NS_PER_MS;
        self.probes.push_back(wire_at_ns);
        if self.probes.len() > 3 {
            self.probes.pop_front();
        }
        use prost::Message;
        let envelope = wire::Envelope {
            schema_version: pilotage_protocol::SCHEMA_VERSION,
            payload: Some(wire::envelope::Payload::Ping(wire::Ping {
                nonce: wire_at_ns,
                sender_sent_at: Some(wire::MonoTimestamp { nanos: wire_at_ns }),
            })),
        };
        Some(ClientAction::SendDatagram(envelope.encode_to_vec()))
    }
}

impl ClientEngine {
    /// Maintains a validity window for held leases without a control command.
    ///
    /// This sends only a rate-limited Ping. It cannot replay an input event or
    /// create motion, a stop command, or a control lease.
    pub fn maintain_control_window(&mut self, now_ns: u64) -> Vec<ClientAction> {
        if self.lanes.is_empty() {
            return Vec::new();
        }
        self.probe_control_window(now_ns)
    }

    /// Whether a held lane can prepare an action at the original input time.
    ///
    /// The host and adapter must still check authority and expiry at delivery.
    #[must_use]
    pub fn control_action_ready(
        &self,
        vehicle_id: u64,
        scope: &str,
        sampled_at_nanos: u64,
    ) -> bool {
        self.holds(vehicle_id, scope)
            && (!self.requires_control_window() || self.window.id_at(sampled_at_nanos) != 0)
    }

    pub(super) fn probe_control_window(&mut self, now_ns: u64) -> Vec<ClientAction> {
        if !self.requires_control_window() {
            return Vec::new();
        }
        self.window.probe(now_ns).into_iter().collect()
    }
}
