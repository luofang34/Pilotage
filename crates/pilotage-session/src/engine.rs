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

mod action_command;
mod frame_ingress;
mod handlers;
mod link_loss;

use pilotage_adapter_api::AdapterCapabilities;
use pilotage_authority::{AuthorityCommand, AuthorityEffect, AuthorityEngine, LinkState};
use pilotage_timing::{MonoTimestamp, StalenessPolicy};

use crate::action::{LinkLossTrigger, SessionAction, SessionOutcome};
use crate::capabilities::scope_pairs;
use crate::clients::ClientRegistry;
use crate::config::SessionConfig;
use crate::liveness::{ExpiredHolder, HolderLiveness};
use crate::message::{ClientKey, DomainEnvelope};
use crate::outbound::OutboundMessage;
use link_loss::LinkLossState;

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
    /// Frame-silence deadlines for every scope that has a holder; drives the
    /// watchdog that releases a holder whose client stops sending while its
    /// connection stays open.
    liveness: HolderLiveness,
    /// Per-vehicle declared link-loss policy selection and which vehicles
    /// currently have a policy engaged (awaiting the recovery activation
    /// condition).
    link_loss: LinkLossState,
    /// Reusable buffer for the watchdog's expiry pass, so the steady-state
    /// tick allocates nothing.
    expired_scratch: Vec<ExpiredHolder>,
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
        let mut scope_count: usize = 0;
        for (vehicle, scope) in scope_pairs(&capabilities) {
            clients.register_scope((vehicle, scope.clone()));
            let effects = authority.handle(AuthorityCommand::RegisterScope { vehicle, scope }, now);
            drop(effects);
            scope_count = scope_count.saturating_add(1);
        }
        let link_loss =
            LinkLossState::from_capabilities(&capabilities, &config.link_loss_overrides);
        Self {
            authority,
            capabilities,
            staleness,
            config,
            clients,
            liveness: HolderLiveness::with_scope_capacity(scope_count),
            link_loss,
            expired_scratch: Vec::with_capacity(scope_count),
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
            DomainEnvelope::Release(release) => {
                self.on_release(client, release, now, &mut actions);
            }
            DomainEnvelope::Frame(frame) => self.on_frame(client, frame, now, &mut actions),
            DomainEnvelope::Ping(ping) => self.on_ping(client, ping, now, &mut actions),
            DomainEnvelope::ProfileActivation(activation) => {
                self.on_profile_activation(client, activation, &mut actions);
            }
            DomainEnvelope::ActionCommand(command) => {
                self.on_action_command(client, command, now, &mut actions);
            }
            DomainEnvelope::Disconnect => self.on_disconnect(client, &mut actions),
        }
        actions.into_outcome()
    }

    /// Advances time-driven state to `now`: offer expiry, then the
    /// holder-silence watchdog.
    ///
    /// Offer expiry is delegated to the embedded [`AuthorityEngine`]. The
    /// watchdog then releases every holder whose frame-silence deadline has
    /// passed by routing a synthetic `HolderLinkChanged { Lost }` through the
    /// same authority path a disconnect uses, so the generation advances (late
    /// straggler frames are fenced) and the vehicle is neutralized exactly once.
    ///
    /// The returned [`SessionOutcome`] carries [`SessionOutcome::dropped`], the
    /// count of actions the per-call cap forced the engine to drop; a non-zero
    /// value is a correctness signal the driver MUST count.
    #[must_use]
    pub fn handle_tick(&mut self, now: MonoTimestamp) -> SessionOutcome {
        let mut actions = Actions::new(self.config.max_actions_per_call);
        let effects = self.authority.expire_due(now);
        self.fan_out_authority(
            effects,
            now,
            LinkLossTrigger::AuthorityRevoked,
            &mut actions,
        );
        // A holder that stopped sending frames while its connection stayed open
        // (a frozen client the QUIC keepalive holds up) is released here. The
        // expiry list reuses the engine's scratch buffer (taken and restored
        // around the loop so the borrow does not pin `self`).
        let mut expired = core::mem::take(&mut self.expired_scratch);
        self.liveness.expire_into(now, &mut expired);
        for holder in expired.drain(..) {
            let effects = self.authority.handle(
                AuthorityCommand::HolderLinkChanged {
                    vehicle: holder.vehicle,
                    scope: holder.scope,
                    principal: holder.principal,
                    state: LinkState::Lost,
                },
                now,
            );
            self.fan_out_authority(effects, now, LinkLossTrigger::HolderSilence, &mut actions);
        }
        self.expired_scratch = expired;
        // Re-drive any scope whose clear the adapter refused, so recovery is not
        // stranded on one failed enactment. Generation-gated inside: a holder
        // change already reverted the pending, so this never crosses a
        // generation (ADR-0010 neutral-activation stays required).
        self.retry_pending_clears(&mut actions);
        actions.into_outcome()
    }

    /// Returns the next time the engine wants a [`SessionEngine::handle_tick`]
    /// call: the earlier of the authority engine's next offer expiry and the
    /// earliest holder-silence deadline.
    #[must_use]
    pub fn next_deadline(&self) -> Option<MonoTimestamp> {
        min_option(
            self.authority.next_deadline(),
            self.liveness.next_deadline(),
        )
    }

    /// Broadcasts each authority effect and keeps holder bookkeeping in sync.
    ///
    /// Every effect that installs or clears an effective holder updates the
    /// registry (so the disconnect path releases exactly the scopes the client
    /// still holds), refreshes or clears the holder-silence watchdog, and emits
    /// the adapter link-loss engagement for the transition after the broadcast:
    /// clients learn the fenced state first, then the vehicle is neutralized.
    /// The engagement rides the uncapped safety lane — a burst of broadcasts
    /// hitting the per-call cap must never swallow the neutralization
    /// (fail-closed, never dropped).
    pub(crate) fn fan_out_authority(
        &mut self,
        effects: Vec<AuthorityEffect>,
        now: MonoTimestamp,
        trigger: LinkLossTrigger,
        actions: &mut Actions,
    ) {
        for effect in effects {
            self.record_holder_change(&effect);
            let transition = self.holder_transition_action(&effect, now, trigger);
            actions.broadcast(OutboundMessage::Authority(effect));
            if let Some(action) = transition {
                match action {
                    SessionAction::EngageLinkLoss { .. } => actions.push_safety(action),
                    _ => actions.push(action),
                }
            }
        }
    }

    /// Refreshes a holder's frame-silence deadline after it sent an accepted
    /// axis-bearing frame; called from the on-frame accept path. Refresh is
    /// in place (no allocation) and only for the recorded holder — a frame
    /// can never create or resurrect a watchdog entry.
    pub(crate) fn note_frame_accepted(
        &mut self,
        vehicle: pilotage_protocol::VehicleId,
        scope: &pilotage_protocol::ScopeId,
        holder: pilotage_protocol::PrincipalId,
        now: MonoTimestamp,
    ) {
        self.liveness.refresh(
            vehicle,
            scope,
            holder,
            now.saturating_add(self.config.holder_silence),
        );
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
        // Link-loss release and its neutralization do not consult `now` (the
        // clear path sets no new deadline); a zero stamp keeps disconnect
        // independent of a clock.
        let now = MonoTimestamp::from_nanos(0);
        for (vehicle, scope) in scopes {
            let effects = self.authority.handle(
                AuthorityCommand::HolderLinkChanged {
                    vehicle,
                    scope,
                    principal,
                    state: LinkState::Lost,
                },
                now,
            );
            self.fan_out_authority(effects, now, LinkLossTrigger::HolderDisconnect, actions);
        }
    }
}

/// The lesser of two optional deadlines, or whichever is present.
fn min_option(a: Option<MonoTimestamp>, b: Option<MonoTimestamp>) -> Option<MonoTimestamp> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (some, None) | (None, some) => some,
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
    /// Count of ordinary (cap-subject) actions in `items`; safety-lane
    /// pushes are excluded so they never consume ordinary budget.
    ordinary: usize,
    cap: usize,
    dropped: usize,
}

impl Actions {
    fn new(cap: usize) -> Self {
        Self {
            items: Vec::new(),
            ordinary: 0,
            cap,
            dropped: 0,
        }
    }

    /// Appends an action unless the per-call cap is already reached.
    pub(crate) fn push(&mut self, action: SessionAction) {
        if self.ordinary >= self.cap {
            self.dropped = self.dropped.wrapping_add(1);
            return;
        }
        self.ordinary = self.ordinary.saturating_add(1);
        self.items.push(action);
    }

    /// Appends a safety-critical action REGARDLESS of the per-call cap.
    ///
    /// The cap exists to bound amplification of untrusted client input;
    /// safety enactments (link-loss engagement) are bounded by the number
    /// of registered scopes, not by client traffic, and dropping one would
    /// leave a vehicle executing its last command with authority already
    /// fenced. A safety action is therefore never droppable behind the
    /// broadcast cap — the vector may exceed the cap by at most the scope
    /// count.
    pub(crate) fn push_safety(&mut self, action: SessionAction) {
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
