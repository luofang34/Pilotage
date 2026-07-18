//! Link-loss policy enactment on the live adapter (ADR-0008, ADR-0010): the
//! actor drives engage/clear to `VehicleAdapter::set_link_loss_policy` and
//! counts a refused enactment as a typed fail-closed fault, never a silent
//! no-op — authority is already fenced when these actions arrive, so an
//! unenacted policy means the vehicle may still be executing its last
//! command with nobody in control.

use pilotage_adapter_api::{LinkLossPolicy, VehicleAdapter};
use pilotage_protocol::VehicleId;
use pilotage_session::SessionAction;
use tracing::{debug, error, warn};

use super::EngineActor;

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
                // The holder was lost; the generation already advanced, so
                // this drives the vehicle to its declared policy state once.
                // The host does not re-transmit.
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
                    Some(policy),
                    "FAIL-CLOSED FAULT: link-loss policy was not enacted; \
                     the vehicle may still be executing its last command",
                );
            }
            SessionAction::ClearLinkLoss { vehicle } => {
                // The recovery conditions held (fresh generation + activation
                // frame); return the vehicle to normal control (ADR-0008's
                // only path back). A failed clear leaves the vehicle
                // neutralized — safe but stuck; counted the same way.
                debug!(
                    vehicle = vehicle.as_u64(),
                    "recovery conditions met; clearing link-loss policy"
                );
                self.set_policy_counting_failure(
                    vehicle,
                    None,
                    "link-loss policy clear failed; vehicle remains neutralized",
                );
            }
            _ => {}
        }
    }

    /// Drives a link-loss policy change to the adapter, counting and
    /// surfacing a refused enactment as a typed fault (never silent).
    fn set_policy_counting_failure(
        &mut self,
        vehicle: VehicleId,
        policy: Option<LinkLossPolicy>,
        fault: &str,
    ) {
        if let Err(error) = self.adapter.set_link_loss_policy(vehicle, policy) {
            self.link_loss_enact_failures = self.link_loss_enact_failures.wrapping_add(1);
            error!(
                vehicle = vehicle.as_u64(),
                ?policy,
                %error,
                failures = self.link_loss_enact_failures,
                fault,
            );
        }
    }
}
