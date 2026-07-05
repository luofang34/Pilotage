//! Per-command handlers for [`AuthorityEngine`] (ADR-0010 state machine).
//!
//! Each handler is a private method returning the ordered effects for one
//! command. They share the invariant that a rejected or no-op command leaves
//! state untouched and returns a single warning or
//! [`AuthorityEffect::CommandRejected`].

use core::time::Duration;

use pilotage_protocol::{Generation, PrincipalId, ScopeId, VehicleId};
use pilotage_timing::MonoTimestamp;

use crate::command::{AuthorityClass, LinkState, OverrideReason};
use crate::effect::{AuthorityEffect, AuthorityWarning, RejectReason};
use crate::engine::{AuthorityEngine, ScopeKey};
use crate::state::{HolderState, ScopeState};

impl AuthorityEngine {
    /// Registers a `(vehicle, scope)`; rejects a duplicate registration.
    pub(super) fn handle_register(
        &mut self,
        vehicle: VehicleId,
        scope: ScopeId,
    ) -> Vec<AuthorityEffect> {
        let key: ScopeKey = (vehicle, scope.clone());
        if self.scopes.contains_key(&key) {
            return vec![AuthorityEffect::CommandRejected {
                vehicle,
                scope,
                reason: RejectReason::ScopeAlreadyRegistered,
            }];
        }
        self.scopes.insert(key, ScopeState::new());
        vec![AuthorityEffect::ScopeRegistered { vehicle, scope }]
    }

    /// Grants an unassigned scope to a principal, advancing the generation.
    pub(super) fn handle_grant(
        &mut self,
        vehicle: VehicleId,
        scope: ScopeId,
        to: PrincipalId,
    ) -> Vec<AuthorityEffect> {
        let Some(state) = self.scopes.get_mut(&(vehicle, scope.clone())) else {
            return reject_unknown(vehicle, scope);
        };
        if let Some(current_holder) = state.effective_holder() {
            return vec![AuthorityEffect::CommandRejected {
                vehicle,
                scope,
                reason: RejectReason::ScopeNotUnassigned { current_holder },
            }];
        }
        state.advance_generation();
        state.holder = HolderState::Held {
            principal: to,
            override_class: None,
        };
        state.link = LinkState::Nominal;
        vec![AuthorityEffect::ScopeLeaseGranted {
            vehicle,
            scope,
            holder: to,
            generation: state.generation,
        }]
    }

    /// Offers a held scope to another principal (handover phase one).
    pub(super) fn handle_offer(
        &mut self,
        vehicle: VehicleId,
        scope: ScopeId,
        from: PrincipalId,
        to: PrincipalId,
        ttl: Duration,
        now: MonoTimestamp,
    ) -> Vec<AuthorityEffect> {
        let Some(state) = self.scopes.get_mut(&(vehicle, scope.clone())) else {
            return reject_unknown(vehicle, scope);
        };
        match &state.holder {
            HolderState::Unassigned => reject(vehicle, scope, RejectReason::ScopeUnassigned),
            HolderState::Offered { .. } => {
                reject(vehicle, scope, RejectReason::OfferAlreadyPending)
            }
            HolderState::Held { principal, .. } if *principal != from => {
                let current_holder = Some(*principal);
                reject(
                    vehicle,
                    scope,
                    RejectReason::NotCurrentHolder {
                        actor: from,
                        current_holder,
                    },
                )
            }
            HolderState::Held { .. } => {
                let expires_at = now.saturating_add(ttl);
                state.holder = HolderState::Offered {
                    from,
                    to,
                    offered_at: now,
                    expires_at,
                };
                vec![AuthorityEffect::ScopeTransferOffered {
                    vehicle,
                    scope,
                    from,
                    to,
                    generation: state.generation,
                    expires_at,
                }]
            }
        }
    }

    /// Accepts a pending offer — the atomic commit of a normal handover.
    ///
    /// Self-fences the offer-expiry-vs-accept race (ADR-0010): an accept whose
    /// `now` is at or past the offer's `expires_at` cannot commit even if the
    /// host has not yet called [`AuthorityEngine::expire_due`]. Such an accept
    /// reverts the scope to `Held(from)` and emits
    /// [`AuthorityEffect::ScopeTransferExpired`], exactly as a scheduled expiry
    /// would, so the outcome does not depend on host call ordering.
    ///
    /// [`AuthorityEngine::expire_due`]: crate::engine::AuthorityEngine::expire_due
    pub(super) fn handle_accept(
        &mut self,
        vehicle: VehicleId,
        scope: ScopeId,
        by: PrincipalId,
        expected_generation: Generation,
        now: MonoTimestamp,
    ) -> Vec<AuthorityEffect> {
        let Some(state) = self.scopes.get_mut(&(vehicle, scope.clone())) else {
            return reject_unknown(vehicle, scope);
        };
        let HolderState::Offered {
            from,
            to,
            expires_at,
            ..
        } = &state.holder
        else {
            return reject(vehicle, scope, RejectReason::NoPendingOffer);
        };
        let (from, to, expires_at) = (*from, *to, *expires_at);
        if now >= expires_at {
            let holder = state.expire_offer().unwrap_or(from);
            return vec![AuthorityEffect::ScopeTransferExpired {
                vehicle,
                scope,
                holder,
                generation: state.generation,
            }];
        }
        if by != to {
            return reject(
                vehicle,
                scope,
                RejectReason::NotOfferRecipient {
                    actor: by,
                    expected: to,
                },
            );
        }
        if expected_generation != state.generation {
            return reject(
                vehicle,
                scope,
                RejectReason::GenerationMismatch {
                    supplied: expected_generation,
                    current: state.generation,
                },
            );
        }
        state.advance_generation();
        state.holder = HolderState::Held {
            principal: to,
            override_class: None,
        };
        state.link = LinkState::Nominal;
        vec![AuthorityEffect::ScopeTransferCommitted {
            vehicle,
            scope,
            from,
            to,
            generation: state.generation,
        }]
    }

    /// Records an "I have control" confirmation; never changes state.
    pub(super) fn handle_confirm_i_have(
        &mut self,
        vehicle: VehicleId,
        scope: ScopeId,
        by: PrincipalId,
    ) -> Vec<AuthorityEffect> {
        let Some(state) = self.scopes.get(&(vehicle, scope.clone())) else {
            return reject_unknown(vehicle, scope);
        };
        let current_holder = state.effective_holder();
        if current_holder == Some(by) {
            // A confirmation from the effective holder is expected and benign;
            // it is audited by the host, not re-emitted as a warning.
            return vec![AuthorityEffect::LinkStateChanged {
                vehicle,
                scope,
                principal: by,
                state: state.link,
            }];
        }
        vec![AuthorityEffect::WarningRaised {
            vehicle,
            scope,
            warning: AuthorityWarning::UnexpectedIHave { by, current_holder },
        }]
    }

    /// Records a "you have control" confirmation; never changes state.
    pub(super) fn handle_confirm_you_have(
        &mut self,
        vehicle: VehicleId,
        scope: ScopeId,
        by: PrincipalId,
    ) -> Vec<AuthorityEffect> {
        let Some(state) = self.scopes.get(&(vehicle, scope.clone())) else {
            return reject_unknown(vehicle, scope);
        };
        let current_holder = state.effective_holder();
        // "You have control" is the previous holder's callout after commit, so
        // the issuer is expected NOT to be the current holder. Any issuer that
        // is the current holder is contradictory and warned.
        if current_holder == Some(by) {
            return vec![AuthorityEffect::WarningRaised {
                vehicle,
                scope,
                warning: AuthorityWarning::UnexpectedYouHave { by, current_holder },
            }];
        }
        vec![AuthorityEffect::LinkStateChanged {
            vehicle,
            scope,
            principal: by,
            state: state.link,
        }]
    }

    /// Voluntarily releases a held scope by its current holder.
    pub(super) fn handle_release(
        &mut self,
        vehicle: VehicleId,
        scope: ScopeId,
        by: PrincipalId,
    ) -> Vec<AuthorityEffect> {
        let Some(state) = self.scopes.get_mut(&(vehicle, scope.clone())) else {
            return reject_unknown(vehicle, scope);
        };
        match state.effective_holder() {
            None => reject(vehicle, scope, RejectReason::ScopeUnassigned),
            Some(holder) if holder != by => reject(
                vehicle,
                scope,
                RejectReason::NotCurrentHolder {
                    actor: by,
                    current_holder: Some(holder),
                },
            ),
            Some(holder) => {
                state.advance_generation();
                state.holder = HolderState::Unassigned;
                state.link = LinkState::Nominal;
                vec![AuthorityEffect::ScopeLeaseRevoked {
                    vehicle,
                    scope,
                    previous_holder: Some(holder),
                    generation: state.generation,
                }]
            }
        }
    }

    /// Administratively revokes a scope, emptying it and advancing generation.
    pub(super) fn handle_revoke(
        &mut self,
        vehicle: VehicleId,
        scope: ScopeId,
        _authority_class: AuthorityClass,
    ) -> Vec<AuthorityEffect> {
        let Some(state) = self.scopes.get_mut(&(vehicle, scope.clone())) else {
            return reject_unknown(vehicle, scope);
        };
        let previous_holder = state.effective_holder();
        state.advance_generation();
        state.holder = HolderState::Unassigned;
        state.link = LinkState::Nominal;
        vec![AuthorityEffect::ScopeLeaseRevoked {
            vehicle,
            scope,
            previous_holder,
            generation: state.generation,
        }]
    }

    /// Applies an emergency override, idempotent for a repeat by the same
    /// holder and class.
    pub(super) fn handle_override(
        &mut self,
        vehicle: VehicleId,
        scope: ScopeId,
        by: PrincipalId,
        authority_class: AuthorityClass,
        reason: OverrideReason,
    ) -> Vec<AuthorityEffect> {
        let Some(state) = self.scopes.get_mut(&(vehicle, scope.clone())) else {
            return reject_unknown(vehicle, scope);
        };
        if let HolderState::Held {
            principal,
            override_class: Some(existing_class),
        } = &state.holder
            && *principal == by
            && *existing_class == authority_class
        {
            return vec![AuthorityEffect::EmergencyOverrideAlreadyEffective {
                vehicle,
                scope,
                holder: by,
                authority_class,
                generation: state.generation,
            }];
        }
        let previous_holder = state.effective_holder();
        state.advance_generation();
        state.holder = HolderState::Held {
            principal: by,
            override_class: Some(authority_class),
        };
        state.link = LinkState::Nominal;
        vec![AuthorityEffect::EmergencyOverrideApplied {
            vehicle,
            scope,
            previous_holder,
            holder: by,
            authority_class,
            reason,
            generation: state.generation,
        }]
    }

    /// Records a holder link change; a lost effective holder releases the
    /// scope (v1 policy).
    pub(super) fn handle_link_changed(
        &mut self,
        vehicle: VehicleId,
        scope: ScopeId,
        principal: PrincipalId,
        link_state: LinkState,
    ) -> Vec<AuthorityEffect> {
        let Some(state) = self.scopes.get_mut(&(vehicle, scope.clone())) else {
            return reject_unknown(vehicle, scope);
        };
        // Only the effective holder's link governs the scope; a report about
        // any other principal is recorded but drives no state change.
        if state.effective_holder() != Some(principal) {
            return vec![AuthorityEffect::LinkStateChanged {
                vehicle,
                scope,
                principal,
                state: link_state,
            }];
        }
        state.link = link_state;
        if link_state != LinkState::Lost {
            return vec![AuthorityEffect::LinkStateChanged {
                vehicle,
                scope,
                principal,
                state: link_state,
            }];
        }
        // v1 link-loss policy: releasing the scope on loss of the effective
        // holder empties it and advances the generation so any late frame from
        // the lost holder is fenced out immediately.
        state.advance_generation();
        state.holder = HolderState::Unassigned;
        state.link = LinkState::Nominal;
        vec![
            AuthorityEffect::LinkStateChanged {
                vehicle,
                scope: scope.clone(),
                principal,
                state: LinkState::Lost,
            },
            AuthorityEffect::HolderLinkLost {
                vehicle,
                scope,
                lost_holder: principal,
                generation: state.generation,
            },
        ]
    }
}

/// Builds the single-effect rejection for an unregistered scope.
fn reject_unknown(vehicle: VehicleId, scope: ScopeId) -> Vec<AuthorityEffect> {
    vec![AuthorityEffect::CommandRejected {
        vehicle,
        scope,
        reason: RejectReason::UnknownScope,
    }]
}

/// Builds a single-effect rejection with the given reason.
fn reject(vehicle: VehicleId, scope: ScopeId, reason: RejectReason) -> Vec<AuthorityEffect> {
    vec![AuthorityEffect::CommandRejected {
        vehicle,
        scope,
        reason,
    }]
}
