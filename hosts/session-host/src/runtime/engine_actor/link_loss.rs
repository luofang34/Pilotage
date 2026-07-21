//! Link-loss policy enactment on the live adapter (ADR-0008, ADR-0010): the
//! actor drives engage/clear to `VehicleAdapter::set_link_loss_policy` PER
//! SCOPE and counts a refused enactment as a typed fail-closed fault, never a
//! silent no-op — authority is already fenced when these actions arrive, so an
//! unenacted policy means the vehicle may still be executing its last command
//! with nobody in control.
//!
//! The recovery LIFECYCLE (Engaged → ClearPending → Cleared) lives in the
//! engine, which owns the authority generation. The actor's job is narrow:
//! enact each `ClearLinkLoss`, and on the adapter's confirmation report it back
//! to the engine — which drops the pending state — and broadcast the
//! client-facing `LinkLossCleared` ack, exactly once and only for a clear the
//! adapter actually took. A refused clear is left engaged-pending; the engine
//! re-emits it each tick until it takes, generation-gated so it can never cross
//! a holder change.

use pilotage_adapter_api::{LinkLossPolicy, VehicleAdapter};
use pilotage_protocol::{Generation, LinkLossCleared, ScopeId, VehicleId};
use pilotage_session::{OutboundMessage, SessionAction};
use tracing::{debug, error, warn};

use super::{EngineActor, MessageClass, to_connection_message};

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
            }
            SessionAction::ClearLinkLoss {
                vehicle,
                scope,
                generation,
            } => {
                // A neutral activation requested this scope's clear (or the tick
                // is retrying a refused one); return the SCOPE to normal control
                // (ADR-0008's only path back). A failed clear leaves the vehicle
                // neutralized — safe but stuck; the engine keeps it pending and
                // re-emits it, so it is counted the same way but not stranded.
                debug!(
                    vehicle = vehicle.as_u64(),
                    scope = scope.as_str(),
                    "recovery conditions met; clearing link-loss policy"
                );
                if self.set_policy_counting_failure(
                    vehicle,
                    &scope,
                    None,
                    "link-loss policy clear failed; vehicle remains neutralized",
                ) {
                    // The adapter took the clear. Report it to the engine, which
                    // drops the pending state and answers whether this matched a
                    // still-pending clear at this generation — so a clear whose
                    // pending a holder change invalidated does NOT ack a recovery
                    // that no longer holds. The ack fires exactly when it does.
                    if self
                        .engine
                        .confirm_link_loss_cleared(vehicle, &scope, generation)
                    {
                        self.broadcast_link_loss_cleared(vehicle, scope, generation);
                    }
                }
            }
            _ => {}
        }
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
