//! The host-side session state machine (ADR-0005, ADR-0006, ADR-0009,
//! ADR-0010).
//!
//! [`SessionEngine`] is pure and sans-IO (ADR-0002): the driver hands it
//! decoded [`DomainEnvelope`]s plus an explicit `now`, and it returns
//! [`SessionAction`]s the driver enacts. All handshake, lease, fencing,
//! staleness, and link-loss decisions live here so the host binary is a thin
//! I/O shell.
//!
//! The staleness check treats client-supplied `sampled_at` as same-clock as
//! the host `now`: a documented loopback-only simplification for increment 0.
//! See [`SessionEngine`] for the full caveat and the RTT/offset follow-up.

mod handlers;

use pilotage_adapter_api::AdapterCapabilities;
use pilotage_authority::{AuthorityCommand, AuthorityEffect, AuthorityEngine, LinkState};
use pilotage_timing::{MonoTimestamp, StalenessPolicy};

use crate::action::{SessionAction, SessionOutcome};
use crate::capabilities::scope_pairs;
use crate::clients::ClientRegistry;
use crate::config::SessionConfig;
use crate::message::{ClientKey, DomainEnvelope};
use crate::outbound::OutboundMessage;

/// A pure host-side session state machine driven by decoded messages and an
/// explicit clock.
///
/// Construct with [`SessionEngine::new`]; drive with
/// [`SessionEngine::handle_client_message`] and [`SessionEngine::handle_tick`];
/// schedule ticks from [`SessionEngine::next_deadline`]. The engine owns an
/// embedded [`AuthorityEngine`] and is authoritative for who holds what right
/// now (ADR-0006).
///
/// # Loopback time simplification (increment 0)
///
/// ADR-0009 forbids comparing a remote endpoint's `MonoTimestamp` against a
/// local one without clock correlation. This engine's staleness check computes
/// frame age as `receive_now - frame.sampled_at` **directly**, which is only
/// valid because the increment-0 loopback runs client and host in ONE process
/// on ONE monotonic clock, so `sampled_at` and `receive_now` share an epoch.
///
/// THIS IS A LOOPBACK-ONLY SHORTCUT. On a real WebTransport link the client's
/// `sampled_at` is in the client's `transport_time` domain and MUST be
/// correlated through [`pilotage_timing::RttEstimator`] /
/// [`pilotage_timing::ClockOffset`] (see [`pilotage_timing::estimated_age`])
/// before comparison. Replacing this direct subtraction with the estimator
/// path is the tracked follow-up; do not ship it to a networked host as-is.
#[derive(Debug)]
pub struct SessionEngine {
    authority: AuthorityEngine,
    capabilities: AdapterCapabilities,
    staleness: StalenessPolicy,
    config: SessionConfig,
    clients: ClientRegistry,
}

impl SessionEngine {
    /// Builds an engine over an adapter's capabilities, registering every
    /// `(vehicle, scope)` the adapter exposes with the authority engine so
    /// leases and frames can target them immediately.
    #[must_use]
    pub fn new(
        capabilities: AdapterCapabilities,
        staleness: StalenessPolicy,
        config: SessionConfig,
    ) -> Self {
        let mut authority = AuthorityEngine::new();
        let mut clients = ClientRegistry::new();
        // Registration time is irrelevant (no command consults `now`); a zero
        // stamp keeps construction free of a clock, per ADR-0002.
        let now = MonoTimestamp::from_nanos(0);
        for (vehicle, scope) in scope_pairs(&capabilities) {
            clients.register_scope((vehicle, scope.clone()));
            let effects = authority.handle(AuthorityCommand::RegisterScope { vehicle, scope }, now);
            drop(effects);
        }
        Self {
            authority,
            capabilities,
            staleness,
            config,
            clients,
        }
    }

    /// Handles one decoded client message at time `now`.
    ///
    /// The returned [`SessionOutcome`] carries the actions plus
    /// [`SessionOutcome::dropped`], the count of actions the per-call cap
    /// ([`SessionConfig::max_actions_per_call`]) forced the engine to drop. A
    /// non-zero drop count is a correctness signal the driver MUST count.
    #[must_use]
    pub fn handle_client_message(
        &mut self,
        client: ClientKey,
        msg: DomainEnvelope,
        now: MonoTimestamp,
    ) -> SessionOutcome {
        let mut actions = Actions::new(self.config.max_actions_per_call);
        match msg {
            DomainEnvelope::Hello(hello) => self.on_hello(client, hello, &mut actions),
            DomainEnvelope::Lease(request) => self.on_lease(client, request, now, &mut actions),
            DomainEnvelope::Frame(frame) => self.on_frame(client, frame, now, &mut actions),
            DomainEnvelope::Ping(ping) => self.on_ping(client, ping, now, &mut actions),
            DomainEnvelope::Disconnect => self.on_disconnect(client, &mut actions),
        }
        actions.into_outcome()
    }

    /// Advances time-driven state (offer expiry) to `now`.
    ///
    /// Delegates expiry entirely to the embedded [`AuthorityEngine`], turning
    /// each expiry effect into an authority broadcast.
    ///
    /// The returned [`SessionOutcome`] carries [`SessionOutcome::dropped`], the
    /// count of broadcasts the per-call cap forced the engine to drop; a
    /// non-zero value is a correctness signal the driver MUST count.
    #[must_use]
    pub fn handle_tick(&mut self, now: MonoTimestamp) -> SessionOutcome {
        let mut actions = Actions::new(self.config.max_actions_per_call);
        let effects = self.authority.expire_due(now);
        self.fan_out_authority(effects, &mut actions);
        actions.into_outcome()
    }

    /// Returns the next time the engine wants a [`SessionEngine::handle_tick`]
    /// call, delegating to the authority engine's earliest offer expiry.
    #[must_use]
    pub fn next_deadline(&self) -> Option<MonoTimestamp> {
        self.authority.next_deadline()
    }

    /// Broadcasts each authority effect and keeps holder bookkeeping in sync.
    ///
    /// Every effect that installs or clears an effective holder updates the
    /// registry so the disconnect path releases exactly the scopes the client
    /// still holds.
    pub(crate) fn fan_out_authority(
        &mut self,
        effects: Vec<AuthorityEffect>,
        actions: &mut Actions,
    ) {
        for effect in effects {
            self.record_holder_change(&effect);
            actions.broadcast(OutboundMessage::Authority(effect));
        }
    }

    /// Updates the registry's holder bookkeeping from one authority effect.
    fn record_holder_change(&mut self, effect: &AuthorityEffect) {
        match effect {
            AuthorityEffect::ScopeLeaseGranted {
                vehicle,
                scope,
                holder,
                generation,
            } => self
                .clients
                .record_hold(*holder, (*vehicle, scope.clone()), *generation),
            AuthorityEffect::ScopeTransferCommitted {
                vehicle,
                scope,
                to,
                generation,
                ..
            } => self
                .clients
                .record_hold(*to, (*vehicle, scope.clone()), *generation),
            AuthorityEffect::EmergencyOverrideApplied {
                vehicle,
                scope,
                holder,
                generation,
                ..
            } => self
                .clients
                .record_hold(*holder, (*vehicle, scope.clone()), *generation),
            AuthorityEffect::ScopeLeaseRevoked {
                vehicle,
                scope,
                generation,
                ..
            }
            | AuthorityEffect::HolderLinkLost {
                vehicle,
                scope,
                generation,
                ..
            } => {
                self.clients
                    .clear_pair(&(*vehicle, scope.clone()), *generation);
            }
            _ => {}
        }
    }

    /// Reports every scope a disconnecting principal held as link-lost.
    ///
    /// The authority engine releases only scopes the principal still
    /// effectively holds (a stale entry after a handover is a benign
    /// `LinkStateChanged`), so this is safe to call with the registry's last
    /// known holdings.
    fn release_on_disconnect(
        &mut self,
        principal: pilotage_protocol::PrincipalId,
        scopes: Vec<crate::clients::ScopePair>,
        actions: &mut Actions,
    ) {
        for (vehicle, scope) in scopes {
            let effects = self.authority.handle(
                AuthorityCommand::HolderLinkChanged {
                    vehicle,
                    scope,
                    principal,
                    state: LinkState::Lost,
                },
                // Link-loss handling does not consult `now`; a zero stamp is
                // sound and keeps disconnect independent of a clock.
                MonoTimestamp::from_nanos(0),
            );
            self.fan_out_authority(effects, actions);
        }
    }
}

/// A bounded accumulator for the actions one engine call may emit.
///
/// Enforces [`SessionConfig::max_actions_per_call`]: once full it silently
/// drops further pushes and records the drop count, mirroring the async
/// layer's bounded-channel discipline where a drop is a counted correctness
/// signal, never silent loss the driver cannot observe.
#[derive(Debug)]
pub(crate) struct Actions {
    items: Vec<SessionAction>,
    cap: usize,
    dropped: usize,
}

impl Actions {
    fn new(cap: usize) -> Self {
        Self {
            items: Vec::new(),
            cap,
            dropped: 0,
        }
    }

    /// Appends an action unless the per-call cap is already reached.
    pub(crate) fn push(&mut self, action: SessionAction) {
        if self.items.len() >= self.cap {
            self.dropped = self.dropped.wrapping_add(1);
            return;
        }
        self.items.push(action);
    }

    /// Appends a unicast send to one client.
    pub(crate) fn send(&mut self, client: ClientKey, envelope: OutboundMessage) {
        self.push(SessionAction::SendToClient { client, envelope });
    }

    /// Appends a broadcast to all clients.
    pub(crate) fn broadcast(&mut self, envelope: OutboundMessage) {
        self.push(SessionAction::Broadcast { envelope });
    }

    /// Consumes the accumulator into the call's [`SessionOutcome`], carrying
    /// the collected actions and the count dropped at the cap.
    fn into_outcome(self) -> SessionOutcome {
        SessionOutcome {
            actions: self.items,
            dropped: self.dropped,
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod cap_tests {
    use super::Actions;
    use crate::action::{CloseReason, SessionAction};
    use crate::message::ClientKey;

    fn close(n: u64) -> SessionAction {
        SessionAction::CloseClient {
            client: ClientKey::new(n),
            reason: CloseReason::DuplicateHello,
        }
    }

    #[test]
    fn actions_are_capped_and_drops_are_counted() {
        let mut actions = Actions::new(2);
        actions.push(close(0));
        actions.push(close(1));
        actions.push(close(2));
        actions.push(close(3));
        let outcome = actions.into_outcome();
        assert_eq!(outcome.dropped, 2);
        assert_eq!(outcome.actions.len(), 2);
    }
}
