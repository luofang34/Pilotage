//! Link-loss policy selection, engagement, and recovery activation
//! (ADR-0008, ADR-0010).
//!
//! Link-loss is PER SCOPE: the engine engages a scope's policy when THAT
//! scope's holder is lost and clears THAT scope independently — engaging or
//! clearing `vehicle.gimbal` never touches `vehicle.motion` on the same
//! vehicle. A scope clears only after a fresh fenced authority generation
//! installed a new holder AND that holder demonstrated the activation
//! condition on THAT scope — an accepted
//! frame reporting every axis the scope declares, all inside the neutral
//! deadband, with no pressed edges. Scope-specificity matters twice over: a
//! neutral frame on an unrelated scope proves nothing about the lost one,
//! and a frame covering only a subset of the declared axes would let an
//! adapter's retained latest-valid values (a previously deflected axis the
//! frame omitted) drive the vehicle the instant the latch clears.
//!
//! The enacted policy is CONFIGURED, not inferred: a vehicle's declared
//! `link_loss_actions` is the menu of supported actions (its order carries
//! no meaning), the session config selects from it per vehicle, and a
//! selection the adapter never declared falls closed to
//! [`LinkLossPolicy::Neutralize`] — the only universally safe floor — as
//! does an unconfigured vehicle.

use pilotage_adapter_api::{
    AdapterCapabilities, LinkLossPolicy, intent_satisfies_neutral_activation,
};
use pilotage_authority::AuthorityEffect;
use pilotage_protocol::{Generation, ScopeId, ScopedControlFrame, VehicleId};
use pilotage_timing::MonoTimestamp;

use super::{Actions, SessionEngine};
use crate::action::{LinkLossTrigger, SessionAction};

/// One scope's link-loss lifecycle (ADR-0010). Recovery is a two-step
/// handshake because clearing the adapter latch is fallible: a neutral
/// activation moves the scope to `ClearPending`, and it returns to normal
/// control only once the adapter confirms the clear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeLinkLoss {
    /// Holder lost; the policy is engaged and the scope awaits a new holder's
    /// neutral-activation frame.
    Engaged,
    /// A neutral activation at `generation` requested the clear, not yet
    /// confirmed by the adapter; retried at THIS generation only. A holder
    /// change reverts it to `Engaged` (the new holder must re-demonstrate
    /// neutral activation), so a stale clear can never cross a generation.
    ClearPending { generation: Generation },
}

/// Per-vehicle configured policy plus the link-loss lifecycle of every scope
/// whose holder was lost. Both vectors are bounded by the capability
/// declaration and sized at construction.
#[derive(Debug)]
pub(crate) struct LinkLossState {
    /// The policy configured and validated for each vehicle.
    selected: Vec<(VehicleId, LinkLossPolicy)>,
    /// The link-loss lifecycle of each scope whose holder loss engaged its
    /// vehicle's policy, keyed by `(vehicle, scope)`.
    engaged: Vec<(VehicleId, ScopeId, ScopeLinkLoss)>,
}

impl LinkLossState {
    /// Resolves each vehicle's policy: the configured override when the
    /// vehicle's declared `link_loss_actions` menu contains it, otherwise
    /// `Neutralize` (the fail-closed floor — also the answer for an
    /// unconfigured vehicle or an empty menu). Declaration order is never
    /// consulted: supported-actions is a menu, not a configuration.
    pub(crate) fn from_capabilities(
        capabilities: &AdapterCapabilities,
        overrides: &[(VehicleId, LinkLossPolicy)],
    ) -> Self {
        let mut selected = Vec::with_capacity(capabilities.vehicles.len());
        let mut scope_count: usize = 0;
        for vehicle in &capabilities.vehicles {
            let configured = overrides
                .iter()
                .find(|(id, _)| *id == vehicle.id)
                .map(|(_, policy)| *policy);
            let policy = match configured {
                Some(policy) if vehicle.link_loss_actions.contains(&policy) => policy,
                _ => LinkLossPolicy::Neutralize,
            };
            selected.push((vehicle.id, policy));
            scope_count = scope_count.saturating_add(vehicle.scopes.len());
        }
        Self {
            selected,
            engaged: Vec::with_capacity(scope_count),
        }
    }

    /// The policy configured for `vehicle`; `Neutralize` for a vehicle the
    /// profile never declared (fail-closed).
    fn policy_for(&self, vehicle: VehicleId) -> LinkLossPolicy {
        self.selected
            .iter()
            .find(|(id, _)| *id == vehicle)
            .map(|(_, policy)| *policy)
            .unwrap_or(LinkLossPolicy::Neutralize)
    }

    /// Records (or re-records) the loss of `scope`, resetting it to `Engaged`.
    /// A re-engagement discards any pending clear: the new loss must recover
    /// through a fresh neutral activation. Idempotent for an engaged scope.
    fn engage_scope(&mut self, vehicle: VehicleId, scope: &ScopeId) {
        match self.entry_mut(vehicle, scope) {
            Some(state) => *state = ScopeLinkLoss::Engaged,
            None => self
                .engaged
                .push((vehicle, scope.clone(), ScopeLinkLoss::Engaged)),
        }
    }

    /// Whether `scope` is engaged and still awaiting its recovery activation —
    /// i.e. not already pending an adapter clear. A neutral frame recovers only
    /// a scope in this state.
    fn is_awaiting_activation(&self, vehicle: VehicleId, scope: &ScopeId) -> bool {
        matches!(self.state(vehicle, scope), Some(ScopeLinkLoss::Engaged))
    }

    /// Transitions an `Engaged` scope to `ClearPending { generation }`. Returns
    /// `true` on the transition (the caller emits one `ClearLinkLoss`); `false`
    /// when the scope is already pending or not engaged.
    fn begin_clear_pending(
        &mut self,
        vehicle: VehicleId,
        scope: &ScopeId,
        generation: Generation,
    ) -> bool {
        let Some(state) = self.entry_mut(vehicle, scope) else {
            return false;
        };
        if matches!(state, ScopeLinkLoss::Engaged) {
            *state = ScopeLinkLoss::ClearPending { generation };
            true
        } else {
            false
        }
    }

    /// Invalidates a pending clear on a holder change: reverts `ClearPending`
    /// to `Engaged`, so the new holder must re-demonstrate neutral activation.
    /// No-op for an `Engaged` or untracked scope.
    fn invalidate_pending(&mut self, vehicle: VehicleId, scope: &ScopeId) {
        let Some(state) = self.entry_mut(vehicle, scope) else {
            return;
        };
        if matches!(state, ScopeLinkLoss::ClearPending { .. }) {
            *state = ScopeLinkLoss::Engaged;
        }
    }

    /// The adapter confirmed the clear at `generation`: drops the scope when it
    /// is still `ClearPending { generation }`. Returns `true` when it removed a
    /// matching pending clear (the caller acks exactly once); `false` when the
    /// pending was already invalidated, superseded, or at a different
    /// generation (the ack is suppressed).
    fn confirm_cleared(
        &mut self,
        vehicle: VehicleId,
        scope: &ScopeId,
        generation: Generation,
    ) -> bool {
        let matched = matches!(
            self.state(vehicle, scope),
            Some(ScopeLinkLoss::ClearPending { generation: g }) if *g == generation
        );
        if matched {
            self.engaged
                .retain(|(v, s, _)| !(*v == vehicle && s == scope));
        }
        matched
    }

    /// Every scope currently pending an adapter clear, so the tick can re-emit
    /// each one's retry `ClearLinkLoss` until the adapter accepts it.
    fn pending_clears(&self) -> Vec<(VehicleId, ScopeId, Generation)> {
        self.engaged
            .iter()
            .filter_map(|(vehicle, scope, state)| match state {
                ScopeLinkLoss::ClearPending { generation } => {
                    Some((*vehicle, scope.clone(), *generation))
                }
                ScopeLinkLoss::Engaged => None,
            })
            .collect()
    }

    fn state(&self, vehicle: VehicleId, scope: &ScopeId) -> Option<&ScopeLinkLoss> {
        self.engaged
            .iter()
            .find(|(v, s, _)| *v == vehicle && s == scope)
            .map(|(_, _, state)| state)
    }

    fn entry_mut(&mut self, vehicle: VehicleId, scope: &ScopeId) -> Option<&mut ScopeLinkLoss> {
        self.engaged
            .iter_mut()
            .find(|(v, s, _)| *v == vehicle && s == scope)
            .map(|(_, _, state)| state)
    }
}

impl SessionEngine {
    /// Updates the frame-silence watchdog for one holder-changing effect and
    /// returns the adapter link-loss action it warrants, if any.
    ///
    /// An effect that installs a holder (grant, committed handover, override)
    /// starts that scope's silence deadline but does NOT clear an engaged
    /// policy — recovery additionally requires the new holder's activation
    /// frame on the lost scope (see
    /// [`SessionEngine::maybe_activate_recovery`]). It DOES invalidate any
    /// clear still pending for that scope: a handover or override advances the
    /// generation with no new engagement, so a stale pending clear must not
    /// un-latch the scope for the new holder before it demonstrates neutral
    /// activation. An effect that clears the holder (release, revoke,
    /// link-lost) stops the watchdog, records the scope as engaged, and emits
    /// that SCOPE's engagement (link-loss is per-scope, so every lost scope
    /// engages independently); the fencing generation has already advanced, so
    /// this is neutralize-after-fence.
    pub(super) fn holder_transition_action(
        &mut self,
        effect: &AuthorityEffect,
        now: MonoTimestamp,
        trigger: LinkLossTrigger,
    ) -> Option<SessionAction> {
        let silence_deadline = now.saturating_add(self.config.holder_silence);
        match effect {
            AuthorityEffect::ScopeLeaseGranted {
                vehicle,
                scope,
                holder,
                ..
            }
            | AuthorityEffect::EmergencyOverrideApplied {
                vehicle,
                scope,
                holder,
                ..
            } => {
                self.link_loss.invalidate_pending(*vehicle, scope);
                self.liveness
                    .mark_active((*vehicle, scope.clone()), *holder, silence_deadline);
                None
            }
            AuthorityEffect::ScopeTransferCommitted {
                vehicle, scope, to, ..
            } => {
                self.link_loss.invalidate_pending(*vehicle, scope);
                self.liveness
                    .mark_active((*vehicle, scope.clone()), *to, silence_deadline);
                None
            }
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
                self.liveness.clear(&(*vehicle, scope.clone()));
                // Link-loss is PER SCOPE: engage this scope and emit ITS
                // engagement, so losing one scope's holder (e.g. releasing
                // vehicle.gimbal) never engages or neutralizes another scope
                // (vehicle.motion) on the same vehicle.
                self.link_loss.engage_scope(*vehicle, scope);
                Some(SessionAction::EngageLinkLoss {
                    vehicle: *vehicle,
                    scope: scope.clone(),
                    generation: *generation,
                    trigger,
                    policy: self.link_loss.policy_for(*vehicle),
                })
            }
            _ => None,
        }
    }

    /// Moves one scope from `Engaged` to `ClearPending` when the accepted frame
    /// satisfies the recovery activation condition ON THAT SCOPE; called from
    /// the frame-accept path, so the fenced-generation and holder-identity
    /// checks have already passed. It emits ONE `ClearLinkLoss` (ordered before
    /// the frame's `ApplyToAdapter` so the adapter un-latches first) but does
    /// NOT remove the engagement marker: the scope stays engaged-pending until
    /// the driver reports the adapter confirmed the clear
    /// ([`SessionEngine::confirm_link_loss_cleared`]). A scope already pending
    /// is left to the tick's retry, not re-activated here.
    pub(super) fn maybe_activate_recovery(
        &mut self,
        frame: &ScopedControlFrame,
        actions: &mut Actions,
    ) {
        let (vehicle, generation) = (frame.vehicle, frame.generation);
        // The latch and its recovery live at the GROUP level: a neutral
        // demonstration on whichever member scope the holder acquired
        // clears the one shared latch — a handover to a sibling scope can
        // never leave the old member orphaned-engaged.
        let group = self
            .authority_pair(vehicle, &frame.scope)
            .map_or_else(|| frame.scope.clone(), |(_, group)| group);
        if !self.link_loss.is_awaiting_activation(vehicle, &group) {
            return;
        }
        // Neutrality is judged against the CONCRETE scope's own
        // advertisement — the family the frame actually speaks.
        let Some(descriptor) =
            crate::capabilities::scope_capability(&self.capabilities, vehicle, &frame.scope)
        else {
            // A scope the capabilities no longer describe cannot prove
            // anything; stay engaged (fail closed).
            return;
        };
        // The gate delivers typed-only frames, so neutral activation is a
        // TYPED demonstration: every commanded component inside the
        // limit-scaled deadband of its advertised capability, with no
        // discrete action riding the frame. An actions-only frame (or a
        // family without a neutral posture) demonstrates nothing.
        let neutral = frame.actions.is_empty()
            && frame.intent.as_ref().is_some_and(|intent| {
                descriptor
                    .intents
                    .iter()
                    .find(|capability| capability.family == intent.family())
                    .is_some_and(|capability| {
                        intent_satisfies_neutral_activation(
                            intent,
                            capability,
                            self.config.activation_deadband_milli,
                        )
                    })
            });
        if !neutral {
            return;
        }
        if self
            .link_loss
            .begin_clear_pending(vehicle, &group, generation)
        {
            // Request the GROUP latch cleared (link-loss is per authority
            // group, so clearing vehicle.gimbal never returns the motion
            // group to control). The engagement marker STAYS until the
            // driver confirms the adapter took the clear, and the client-facing
            // LinkLossCleared ack is emitted by the driver only then — so the
            // recovering client never resumes on a clear the vehicle never
            // enacted. Safety critical, so it is never dropped behind the cap.
            actions.push_safety(SessionAction::ClearLinkLoss {
                vehicle,
                scope: group.clone(),
                generation,
                retry: false,
            });
        }
    }

    /// Re-emits a retry `ClearLinkLoss` for every scope still pending an adapter
    /// clear (ADR-0008 recovery must not strand on one refused enactment).
    /// Called each tick: a clear the adapter took is confirmed and dropped
    /// before the next tick, so this fires only for a genuinely-refused clear,
    /// and only at the pending generation — a holder change already reverted it
    /// to `Engaged`, so a stale clear can never cross a generation.
    pub(super) fn retry_pending_clears(&mut self, actions: &mut Actions) {
        for (vehicle, scope, generation) in self.link_loss.pending_clears() {
            actions.push_safety(SessionAction::ClearLinkLoss {
                vehicle,
                scope,
                generation,
                retry: true,
            });
        }
    }

    /// The driver reports the adapter confirmed a scope's clear at `generation`.
    /// Returns whether it matched a still-pending clear at that generation — the
    /// driver broadcasts the `LinkLossCleared` ack exactly when it does, and
    /// suppresses it otherwise (a clear whose pending was invalidated by a
    /// holder change, or superseded, must not ack a recovery that did not hold).
    pub fn confirm_link_loss_cleared(
        &mut self,
        vehicle: VehicleId,
        scope: &ScopeId,
        generation: Generation,
    ) -> bool {
        self.link_loss.confirm_cleared(vehicle, scope, generation)
    }

    /// Test-only: drives one authority effect through the real fan-out path
    /// (holder-transition + link-loss handling) and returns the resulting
    /// actions. The session's client API cannot yet trigger a committed
    /// handover or an emergency override (increment-0), so their link-loss
    /// races are exercised by feeding the effect the authority engine emits.
    #[cfg(test)]
    pub(crate) fn apply_authority_effect_for_test(
        &mut self,
        effect: AuthorityEffect,
        now: MonoTimestamp,
    ) -> crate::SessionOutcome {
        let mut actions = super::Actions::new(self.config.max_actions_per_call);
        self.fan_out_authority(
            vec![effect],
            now,
            LinkLossTrigger::AuthorityRevoked,
            &mut actions,
        );
        actions.into_outcome()
    }
}

#[cfg(test)]
mod tests;
