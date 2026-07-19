//! Link-loss policy selection, engagement, and recovery activation
//! (ADR-0008, ADR-0010).
//!
//! The engine engages a vehicle's link-loss policy when a scope's holder is
//! lost and clears it only after EVERY lost scope has recovered: for each,
//! a fresh fenced authority generation installed a new holder AND that
//! holder demonstrated the activation condition on THAT scope — an accepted
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
    AdapterCapabilities, LinkLossPolicy, payload_satisfies_neutral_activation,
};
use pilotage_authority::AuthorityEffect;
use pilotage_protocol::{ControlPayload, LogicalAxisId, ScopeId, VehicleId};
use pilotage_timing::MonoTimestamp;

use super::{Actions, SessionEngine};
use crate::action::{LinkLossTrigger, SessionAction};

/// Per-vehicle configured policy plus which scopes currently have their
/// vehicle's policy engaged. Both vectors are bounded by the capability
/// declaration and sized at construction.
#[derive(Debug)]
pub(crate) struct LinkLossState {
    /// The policy configured and validated for each vehicle.
    selected: Vec<(VehicleId, LinkLossPolicy)>,
    /// Scopes whose holder loss engaged (or would keep engaged) their
    /// vehicle's policy, each awaiting scope-specific recovery activation.
    engaged: Vec<(VehicleId, ScopeId)>,
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

    /// Records the loss of `scope` on `vehicle`. Idempotent.
    fn engage_scope(&mut self, vehicle: VehicleId, scope: &ScopeId) {
        if !self.is_scope_engaged(vehicle, scope) {
            self.engaged.push((vehicle, scope.clone()));
        }
    }

    /// Whether this exact scope's loss is awaiting recovery activation.
    fn is_scope_engaged(&self, vehicle: VehicleId, scope: &ScopeId) -> bool {
        self.engaged
            .iter()
            .any(|(v, s)| *v == vehicle && s == scope)
    }

    /// Whether any scope keeps `vehicle`'s policy engaged.
    fn vehicle_engaged(&self, vehicle: VehicleId) -> bool {
        self.engaged.iter().any(|(v, _)| *v == vehicle)
    }

    /// Clears one scope's engagement marker. Idempotent.
    fn clear_scope(&mut self, vehicle: VehicleId, scope: &ScopeId) {
        self.engaged.retain(|(v, s)| !(*v == vehicle && s == scope));
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
    /// [`SessionEngine::maybe_activate_recovery`]). An effect that clears
    /// the holder (release, revoke, link-lost) stops the watchdog, records
    /// the scope as engaged, and — on the vehicle's FIRST engaged scope —
    /// emits the engagement; the fencing generation has already advanced,
    /// so this is neutralize-after-fence. Later scope losses on an
    /// already-engaged vehicle record their scope without re-emitting (the
    /// adapter is already in its policy state).
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
                self.liveness
                    .mark_active((*vehicle, scope.clone()), *holder, silence_deadline);
                None
            }
            AuthorityEffect::ScopeTransferCommitted {
                vehicle, scope, to, ..
            } => {
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
                let first = !self.link_loss.vehicle_engaged(*vehicle);
                self.link_loss.engage_scope(*vehicle, scope);
                if !first {
                    return None;
                }
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

    /// Clears one scope's engagement when the accepted frame satisfies the
    /// recovery activation condition ON THAT SCOPE; called from the
    /// frame-accept path, so the fenced-generation and holder-identity
    /// checks have already passed. The vehicle's `ClearLinkLoss` is emitted
    /// only when its LAST engaged scope recovers, ordered before the
    /// frame's `ApplyToAdapter` so the adapter un-latches first.
    pub(super) fn maybe_activate_recovery(
        &mut self,
        vehicle: VehicleId,
        scope: &ScopeId,
        payload: &ControlPayload,
        actions: &mut Actions,
    ) {
        if !self.link_loss.is_scope_engaged(vehicle, scope) {
            return;
        }
        let Some(declared) = declared_axes(&self.capabilities, vehicle, scope) else {
            // A scope the capabilities no longer describe cannot prove
            // anything; stay engaged (fail closed).
            return;
        };
        if !payload_satisfies_neutral_activation(
            payload,
            declared,
            self.config.activation_deadband_milli,
        ) {
            return;
        }
        self.link_loss.clear_scope(vehicle, scope);
        if !self.link_loss.vehicle_engaged(vehicle) {
            actions.push(SessionAction::ClearLinkLoss { vehicle });
        }
    }
}

/// The axes `scope` declares on `vehicle`, from the adapter's capability
/// report.
fn declared_axes<'a>(
    capabilities: &'a AdapterCapabilities,
    vehicle: VehicleId,
    scope: &ScopeId,
) -> Option<&'a [LogicalAxisId]> {
    capabilities
        .vehicles
        .iter()
        .find(|descriptor| descriptor.id == vehicle)?
        .scopes
        .iter()
        .find(|descriptor| descriptor.scope == *scope)
        .map(|descriptor| descriptor.axes.as_slice())
}
