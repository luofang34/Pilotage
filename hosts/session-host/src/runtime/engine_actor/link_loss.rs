//! Link-loss policy enactment on the live adapter (ADR-0008, ADR-0010): the
//! actor drives engage/clear to `VehicleAdapter::set_link_loss_policy` PER
//! SCOPE and counts a refused enactment as a typed fail-closed fault, never a
//! silent no-op — authority is already fenced when these actions arrive, so an
//! unenacted policy means the vehicle may still be executing its last command
//! with nobody in control.
//!
//! The client-facing `LinkLossCleared` recovery ack is broadcast HERE, and
//! only after the adapter CONFIRMS the clear: the pure engine cannot observe
//! the fallible adapter result, so gating the ack on a successful clear is the
//! actor's job. A failed clear leaves the vehicle neutralized and emits no
//! ack, so the recovering client keeps neutralizing rather than resuming on a
//! clear the vehicle never enacted.

use pilotage_adapter_api::{LinkLossPolicy, VehicleAdapter};
use pilotage_protocol::{Generation, LinkLossCleared, ScopeId, VehicleId};
use pilotage_session::{OutboundMessage, SessionAction};
use tracing::{debug, error, warn};

use super::{EngineActor, MessageClass, to_connection_message};

/// A recovery clear the engine already un-engaged but whose adapter
/// enactment was refused, held for the actor to retry until it takes.
#[derive(Debug, Clone)]
pub(super) struct PendingClear {
    vehicle: VehicleId,
    scope: ScopeId,
    /// The recovered generation the eventual `LinkLossCleared` ack must echo.
    generation: Generation,
}

impl<A: VehicleAdapter> EngineActor<A> {
    /// Enacts one `EngageLinkLoss` / `ClearLinkLoss` action on the adapter.
    pub(super) fn enact_link_loss(&mut self, action: SessionAction) {
        match action {
            SessionAction::EngageLinkLoss {
                vehicle,
                scope,
                generation,
                trigger,
                policy,
            } => {
                // This scope's holder was lost; the generation already
                // advanced, so this drives the vehicle to its declared policy
                // state for THIS scope once. The host does not re-transmit.
                warn!(
                    vehicle = vehicle.as_u64(),
                    scope = scope.as_str(),
                    generation = generation.as_u64(),
                    ?trigger,
                    ?policy,
                    "holder lost; engaging link-loss policy"
                );
                self.set_policy_counting_failure(
                    vehicle,
                    &scope,
                    Some(policy),
                    "FAIL-CLOSED FAULT: link-loss policy was not enacted; \
                     the vehicle may still be executing its last command",
                );
                // A fresh engagement supersedes any deferred clear still
                // pending for THIS scope: without this, a retry of that stale
                // clear would un-latch the scope we just re-engaged (and ack a
                // recovery that never happened).
                self.pending_clears
                    .retain(|pending| !(pending.vehicle == vehicle && pending.scope == scope));
            }
            SessionAction::ClearLinkLoss {
                vehicle,
                scope,
                generation,
            } => {
                // The recovery conditions held on this scope (fresh generation
                // + activation frame); return the SCOPE to normal control
                // (ADR-0008's only path back). A failed clear leaves the
                // vehicle neutralized — safe but stuck; counted the same way.
                debug!(
                    vehicle = vehicle.as_u64(),
                    scope = scope.as_str(),
                    "recovery conditions met; clearing link-loss policy"
                );
                let cleared = self.set_policy_counting_failure(
                    vehicle,
                    &scope,
                    None,
                    "link-loss policy clear failed; vehicle remains neutralized",
                );
                // A fresh recovery supersedes any earlier deferred retry for
                // this scope.
                self.pending_clears
                    .retain(|pending| !(pending.vehicle == vehicle && pending.scope == scope));
                if cleared {
                    // ONLY after the adapter confirms the clear do we tell the
                    // client its scope recovered, so it never resumes on a clear
                    // the vehicle never enacted.
                    self.broadcast_link_loss_cleared(vehicle, scope, generation);
                } else {
                    // The engine already dropped the engaged marker, so no
                    // later neutral frame will re-emit this clear. Hold it and
                    // retry every tick until the adapter accepts it, THEN ack
                    // exactly once — a refused clear must not strand recovery.
                    self.pending_clears.push(PendingClear {
                        vehicle,
                        scope,
                        generation,
                    });
                }
            }
            _ => {}
        }
    }

    /// Retries every deferred recovery clear once. The FIRST acceptance of a
    /// scope's clear broadcasts its `LinkLossCleared` ack and drops it; a
    /// still-refused clear stays pending (the vehicle remains neutralized —
    /// fail-closed — and the client keeps neutralizing). Retries are not
    /// re-counted as faults: the refusal was already counted when the clear
    /// was first attempted.
    pub(super) fn retry_pending_clears(&mut self) {
        if self.pending_clears.is_empty() {
            return;
        }
        let mut still_pending = Vec::with_capacity(self.pending_clears.len());
        for pending in std::mem::take(&mut self.pending_clears) {
            match self
                .adapter
                .set_link_loss_policy(pending.vehicle, &pending.scope, None)
            {
                Ok(()) => self.broadcast_link_loss_cleared(
                    pending.vehicle,
                    pending.scope.clone(),
                    pending.generation,
                ),
                Err(error) => {
                    debug!(
                        vehicle = pending.vehicle.as_u64(),
                        scope = pending.scope.as_str(),
                        %error,
                        "deferred link-loss clear still refused; will retry next tick"
                    );
                    still_pending.push(pending);
                }
            }
        }
        self.pending_clears = still_pending;
    }

    /// Broadcasts a scope's `LinkLossCleared` recovery ack on the reliable
    /// authority stream — the signal that lets the recovering client resume.
    fn broadcast_link_loss_cleared(
        &mut self,
        vehicle: VehicleId,
        scope: ScopeId,
        generation: Generation,
    ) {
        let envelope = OutboundMessage::LinkLossCleared(LinkLossCleared {
            vehicle,
            scope,
            generation,
        });
        self.broadcast(
            to_connection_message(&envelope),
            MessageClass::AuthorityBroadcast,
        );
    }

    /// Drives a per-scope link-loss policy change to the adapter, counting and
    /// surfacing a refused enactment as a typed fault (never silent). Returns
    /// `true` when the adapter enacted the change, `false` on a counted fault.
    fn set_policy_counting_failure(
        &mut self,
        vehicle: VehicleId,
        scope: &ScopeId,
        policy: Option<LinkLossPolicy>,
        fault: &str,
    ) -> bool {
        match self.adapter.set_link_loss_policy(vehicle, scope, policy) {
            Ok(()) => true,
            Err(error) => {
                self.link_loss_enact_failures = self.link_loss_enact_failures.wrapping_add(1);
                error!(
                    vehicle = vehicle.as_u64(),
                    scope = scope.as_str(),
                    ?policy,
                    %error,
                    failures = self.link_loss_enact_failures,
                    fault,
                );
                false
            }
        }
    }
}
