//! The authority engine: a sans-IO state machine over `(vehicle, scope)`
//! authority (ADR-0006, ADR-0010, ADR-0012).
//!
//! [`AuthorityEngine`] owns per-scope state and turns [`AuthorityCommand`]s
//! into ordered [`AuthorityEffect`]s. It reads no clock and performs no I/O:
//! every time-dependent operation takes an explicit `now`. The single atomic
//! commit for a normal handover is [`AuthorityCommand::Accept`]; confirmations
//! never gate a transfer.

mod handlers;

use std::collections::BTreeMap;

use pilotage_protocol::{Generation, ScopeId, VehicleId};
use pilotage_timing::MonoTimestamp;

use crate::command::AuthorityCommand;
use crate::effect::{AuthorityEffect, FrameVerdict};
use crate::state::{HolderState, ScopeState};

/// The key identifying one authority state machine.
///
/// Authority is maintained per `(vehicle, scope)` pair (ADR-0006). `ScopeId`
/// is an owned string, so the key is cloned into the map on registration.
type ScopeKey = (VehicleId, ScopeId);

/// A sans-IO engine holding authority state for many `(vehicle, scope)` pairs.
///
/// Construct with [`AuthorityEngine::new`], register scopes with
/// [`AuthorityCommand::RegisterScope`], then drive it with
/// [`AuthorityEngine::handle`]. Poll [`AuthorityEngine::next_deadline`] for the
/// earliest pending offer expiry and call [`AuthorityEngine::expire_due`] when
/// that deadline is reached.
#[derive(Debug, Default)]
pub struct AuthorityEngine {
    scopes: BTreeMap<ScopeKey, ScopeState>,
}

impl AuthorityEngine {
    /// Creates an engine with no registered scopes.
    #[must_use]
    pub fn new() -> Self {
        Self {
            scopes: BTreeMap::new(),
        }
    }

    /// Handles one command at time `now`, returning the ordered effects.
    ///
    /// `now` is consulted by commands whose semantics depend on time:
    /// [`AuthorityCommand::Offer`] stamps the expiry, and
    /// [`AuthorityCommand::Accept`] self-fences against an offer that has
    /// already expired at `now` (so an accept arriving after the TTL cannot
    /// commit even if the host has not yet called
    /// [`AuthorityEngine::expire_due`]). All other commands ignore it. The
    /// returned vector is never empty: every
    /// command produces at least one effect (a mutation, a warning, or a
    /// [`AuthorityEffect::CommandRejected`]).
    pub fn handle(
        &mut self,
        command: AuthorityCommand,
        now: MonoTimestamp,
    ) -> Vec<AuthorityEffect> {
        match command {
            AuthorityCommand::RegisterScope { vehicle, scope } => {
                self.handle_register(vehicle, scope)
            }
            AuthorityCommand::Grant { vehicle, scope, to } => self.handle_grant(vehicle, scope, to),
            AuthorityCommand::Offer {
                vehicle,
                scope,
                from,
                to,
                ttl,
            } => self.handle_offer(vehicle, scope, from, to, ttl, now),
            AuthorityCommand::Accept {
                vehicle,
                scope,
                by,
                expected_generation,
            } => self.handle_accept(vehicle, scope, by, expected_generation, now),
            AuthorityCommand::ConfirmIHave { vehicle, scope, by } => {
                self.handle_confirm_i_have(vehicle, scope, by)
            }
            AuthorityCommand::ConfirmYouHave { vehicle, scope, by } => {
                self.handle_confirm_you_have(vehicle, scope, by)
            }
            AuthorityCommand::Release { vehicle, scope, by } => {
                self.handle_release(vehicle, scope, by)
            }
            AuthorityCommand::Revoke {
                vehicle,
                scope,
                authority_class,
            } => self.handle_revoke(vehicle, scope, authority_class),
            AuthorityCommand::EmergencyOverride {
                vehicle,
                scope,
                by,
                authority_class,
                reason,
            } => self.handle_override(vehicle, scope, by, authority_class, reason),
            AuthorityCommand::HolderLinkChanged {
                vehicle,
                scope,
                principal,
                state,
            } => self.handle_link_changed(vehicle, scope, principal, state),
        }
    }

    /// Returns the earliest pending offer expiry across all scopes, if any.
    ///
    /// The embedding host uses this to schedule the next call to
    /// [`AuthorityEngine::expire_due`]. Returns `None` when no offer is
    /// pending.
    #[must_use]
    pub fn next_deadline(&self) -> Option<MonoTimestamp> {
        self.scopes
            .values()
            .filter_map(|state| match &state.holder {
                HolderState::Offered { expires_at, .. } => Some(*expires_at),
                _ => None,
            })
            .min()
    }

    /// Expires every pending offer whose deadline is at or before `now`.
    ///
    /// Each expiry returns its scope to `Held(from)` without advancing the
    /// generation (no transfer occurred) and emits
    /// [`AuthorityEffect::ScopeTransferExpired`]. Effects are ordered by
    /// `(vehicle, scope)` key for determinism.
    pub fn expire_due(&mut self, now: MonoTimestamp) -> Vec<AuthorityEffect> {
        let mut effects = Vec::new();
        for ((vehicle, scope), state) in &mut self.scopes {
            if let HolderState::Offered { expires_at, .. } = &state.holder
                && *expires_at <= now
                && let Some(holder) = state.expire_offer()
            {
                effects.push(AuthorityEffect::ScopeTransferExpired {
                    vehicle: *vehicle,
                    scope: scope.clone(),
                    holder,
                    generation: state.generation,
                });
            }
        }
        effects
    }

    /// Verifies a control frame against current authority (ADR-0006).
    ///
    /// A frame is [`FrameVerdict::Accepted`] only when the scope is registered,
    /// has an effective holder, and the frame's `generation` equals the
    /// scope's current generation. During a pending offer the offerer remains
    /// the effective holder, so its frames at the current generation are
    /// accepted; frames at any other generation are fenced out.
    #[must_use]
    pub fn verify_frame(
        &self,
        vehicle: VehicleId,
        scope: &ScopeId,
        generation: Generation,
    ) -> FrameVerdict {
        let Some(state) = self.scopes.get(&(vehicle, scope.clone())) else {
            return FrameVerdict::RejectedUnknownScope;
        };
        if state.effective_holder().is_none() {
            return FrameVerdict::RejectedNoHolder;
        }
        if generation != state.generation {
            return FrameVerdict::RejectedStaleGeneration {
                current: state.generation,
            };
        }
        FrameVerdict::Accepted
    }

    /// Borrows the state for a registered scope, for tests and internal use.
    #[cfg(test)]
    pub(crate) fn scope_state(&self, vehicle: VehicleId, scope: &ScopeId) -> Option<&ScopeState> {
        self.scopes.get(&(vehicle, scope.clone()))
    }
}
