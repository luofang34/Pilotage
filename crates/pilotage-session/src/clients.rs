//! Per-client registry and holder bookkeeping for the session engine.
//!
//! The engine assigns each connection a stable `SessionId`/`PrincipalId` at
//! `ClientHello` and tracks which scopes each principal effectively holds, so a
//! disconnect can release exactly those scopes (ADR-0010 link loss) without
//! spraying the authority engine with lost-link reports for scopes the client
//! never held.

use std::collections::{BTreeMap, BTreeSet};

use pilotage_protocol::{
    Generation, PrincipalId, ScopeHolderSnapshot, ScopeId, SessionId, VehicleId,
};

use crate::message::ClientKey;

/// A `(vehicle, scope)` pair, the unit of authority the engine tracks.
pub(crate) type ScopePair = (VehicleId, ScopeId);

/// Identity the engine assigned to one connection at handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClientState {
    /// Session identity assigned to this connection.
    pub(crate) session: SessionId,
    /// Principal identity assigned to this connection.
    pub(crate) principal: PrincipalId,
    /// The client's last announced control-profile activation (INPUT-01):
    /// the traceability record binding the `activation_revision` its frames
    /// carry to the profile identity, document revision, and content digest.
    pub(crate) active_profile: Option<pilotage_protocol::ProfileActivation>,
}

/// Registry of connected clients and the scopes each principal holds.
///
/// Holder bookkeeping is derived from authority effects, never guessed: a
/// grant/commit/override that installs a principal records the scope for it,
/// and any effect that empties or reassigns a scope removes it from the prior
/// holder. This keeps the disconnect path precise even after handovers.
#[derive(Debug, Default)]
pub(crate) struct ClientRegistry {
    clients: BTreeMap<ClientKey, ClientState>,
    held: BTreeMap<PrincipalId, BTreeSet<ScopePair>>,
    holders: BTreeMap<ScopePair, HolderRecord>,
    /// The CONCRETE member scope each group lease was acquired for. A
    /// holder commands only the member it leased — group authority is
    /// exclusive across siblings, never a license to drive them all.
    /// Cleared by every holder-changing effect (fail closed) and set only
    /// by the lease path that knows the requested scope.
    members: BTreeMap<ScopePair, ScopeId>,
    next_session: u64,
    next_principal: u64,
}

/// Current holder and fencing generation for one registered scope, derived
/// wholly from authority effects.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HolderRecord {
    holder: Option<PrincipalId>,
    generation: Generation,
}

impl ClientRegistry {
    /// Creates an empty registry whose id counters start at zero.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Seeds a freshly registered scope as unassigned at generation zero, so a
    /// client welcomed before any lease still sees the scope in its snapshot.
    pub(crate) fn register_scope(&mut self, pair: ScopePair) {
        self.holders.entry(pair).or_insert(HolderRecord {
            holder: None,
            generation: Generation::new(0),
        });
    }

    /// Returns whether the scope is registered (known to the host).
    pub(crate) fn is_registered(&self, pair: &ScopePair) -> bool {
        self.holders.contains_key(pair)
    }

    /// Returns the current fencing generation for a registered scope.
    pub(crate) fn generation_of(&self, pair: &ScopePair) -> Option<Generation> {
        self.holders.get(pair).map(|record| record.generation)
    }

    /// Returns the current effective holder of a registered scope, if any.
    pub(crate) fn holder_of(&self, pair: &ScopePair) -> Option<PrincipalId> {
        self.holders.get(pair).and_then(|record| record.holder)
    }

    /// Snapshots every tracked scope's holder and generation, ordered by
    /// `(vehicle, scope)` for deterministic `ServerWelcome` output.
    pub(crate) fn scope_holders(&self) -> Vec<ScopeHolderSnapshot> {
        self.holders
            .iter()
            .map(|((vehicle, scope), record)| ScopeHolderSnapshot {
                vehicle: *vehicle,
                scope: scope.clone(),
                holder: record.holder,
                generation: record.generation,
            })
            .collect()
    }

    /// Returns the state for a welcomed client, if any.
    pub(crate) fn get(&self, client: ClientKey) -> Option<&ClientState> {
        self.clients.get(&client)
    }

    /// Returns whether the client has completed the handshake.
    pub(crate) fn is_welcomed(&self, client: ClientKey) -> bool {
        self.clients.contains_key(&client)
    }

    /// Assigns a fresh session/principal identity to a new connection.
    ///
    /// Id counters use `wrapping_add(1)` so a very long-lived host process
    /// cannot panic on overflow; collisions after a full `u64` wrap are not a
    /// concern within any realistic session lifetime.
    pub(crate) fn welcome(&mut self, client: ClientKey) -> ClientState {
        let session = SessionId::new(self.next_session);
        let principal = PrincipalId::new(self.next_principal);
        self.next_session = self.next_session.wrapping_add(1);
        self.next_principal = self.next_principal.wrapping_add(1);
        let state = ClientState {
            session,
            principal,
            active_profile: None,
        };
        self.clients.insert(client, state.clone());
        state
    }

    /// Records the client's announced control-profile activation (INPUT-01).
    /// Returns false when the client has not completed the handshake.
    pub(crate) fn record_profile_activation(
        &mut self,
        client: ClientKey,
        activation: pilotage_protocol::ProfileActivation,
    ) -> bool {
        match self.clients.get_mut(&client) {
            Some(state) => {
                state.active_profile = Some(activation);
                true
            }
            None => false,
        }
    }

    /// The client's last announced control-profile activation, if any.
    pub(crate) fn active_profile(
        &self,
        client: ClientKey,
    ) -> Option<&pilotage_protocol::ProfileActivation> {
        self.clients
            .get(&client)
            .and_then(|state| state.active_profile.as_ref())
    }

    /// Removes a client on disconnect, returning the scopes its principal still
    /// held so the caller can issue the matching link-loss reports.
    pub(crate) fn remove(&mut self, client: ClientKey) -> Option<(PrincipalId, Vec<ScopePair>)> {
        let state = self.clients.remove(&client)?;
        let scopes = self
            .held
            .remove(&state.principal)
            .map(|set| set.into_iter().collect())
            .unwrap_or_default();
        Some((state.principal, scopes))
    }

    /// Records that `holder` now holds `pair` at `generation`, clearing any
    /// prior holder of the same pair. The leased MEMBER is cleared here
    /// (fail closed): only the lease path knows the requested concrete
    /// scope and re-binds it after the grant's effects apply.
    pub(crate) fn record_hold(
        &mut self,
        holder: PrincipalId,
        pair: ScopePair,
        generation: Generation,
    ) {
        self.detach_pair(&pair);
        self.members.remove(&pair);
        self.held.entry(holder).or_default().insert(pair.clone());
        self.holders.insert(
            pair,
            HolderRecord {
                holder: Some(holder),
                generation,
            },
        );
    }

    /// Binds the group lease to the concrete member scope it was acquired
    /// for; frames and actions on any sibling are refused.
    pub(crate) fn set_held_member(&mut self, pair: ScopePair, member: ScopeId) {
        self.members.insert(pair, member);
    }

    /// The concrete member scope the group's current lease was acquired
    /// for, if a lease named one.
    pub(crate) fn held_member(&self, pair: &ScopePair) -> Option<&ScopeId> {
        self.members.get(pair)
    }

    /// Records that `pair` is no longer held by anyone, at `generation`.
    pub(crate) fn clear_pair(&mut self, pair: &ScopePair, generation: Generation) {
        self.detach_pair(pair);
        self.members.remove(pair);
        self.holders.insert(
            pair.clone(),
            HolderRecord {
                holder: None,
                generation,
            },
        );
    }

    /// Removes `pair` from every principal's held set without touching the
    /// holder snapshot.
    fn detach_pair(&mut self, pair: &ScopePair) {
        self.held.retain(|_, set| {
            set.remove(pair);
            !set.is_empty()
        });
    }
}
