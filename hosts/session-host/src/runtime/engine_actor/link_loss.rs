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
use pilotage_protocol::{LinkLossCleared, ScopeId, VehicleId};
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
                // ONLY after the adapter confirms the clear do we tell the
                // client its scope recovered, so it never resumes live control
                // on a clear the vehicle never actually enacted. A failed clear
                // broadcasts nothing — the client keeps neutralizing.
                if cleared {
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
            }
            _ => {}
        }
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
