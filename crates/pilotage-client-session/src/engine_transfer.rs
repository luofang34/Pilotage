//! The engine's half of a cooperative handover: accepting the offer
//! that answers its own ask, opening the lane on the commit, closing it
//! when authority departs, and falling back to a plain lease when the
//! scope frees up mid-ask.

use pilotage_protocol::wire;

use crate::action::{ClientAction, ModuleEvent};
use crate::bootstrap;
use crate::control::ControlLane;
use crate::engine::ClientEngine;

impl ClientEngine {
    /// Moves a pending takeover along, and closes a lane whose authority
    /// went to someone else — whichever way it went.
    pub(crate) fn on_transfer_progress(
        &mut self,
        event: &wire::AuthorityEvent,
        principal_id: u64,
    ) -> Vec<ClientAction> {
        match event.event.as_ref() {
            Some(wire::authority_event::Event::ScopeTransferOffered(offered)) => {
                self.on_transfer_offered(offered, principal_id)
            }
            Some(wire::authority_event::Event::ScopeTransferCommitted(committed)) => {
                self.on_transfer_committed(committed, principal_id)
            }
            Some(wire::authority_event::Event::ScopeLeaseRevoked(revoked)) => {
                self.on_scope_freed(revoked)
            }
            _ => Vec::new(),
        }
    }

    /// A revocation lands two ways. This engine's own lane revoked — a
    /// silence watchdog, an override — closes the lane and says so: an
    /// operator must never work a control surface the host already took
    /// back. A stranger's revocation while this engine's ask is pending
    /// frees the scope, so the ask becomes an ordinary lease request.
    fn on_scope_freed(&mut self, revoked: &wire::ScopeLeaseRevoked) -> Vec<ClientAction> {
        let vehicle = revoked.vehicle.as_ref().map_or(0, |v| v.value);
        let scope = revoked
            .scope
            .as_ref()
            .map(|s| s.value.clone())
            .unwrap_or_default();
        if self
            .lane
            .as_ref()
            .is_some_and(|lane| lane.vehicle_id() == vehicle && lane.scope() == scope)
        {
            let generation = revoked.generation.as_ref().map_or(0, |g| g.value);
            self.lane = None;
            return vec![ClientAction::Emit(ModuleEvent::Lease(
                wire::LeaseResponse {
                    vehicle: Some(wire::VehicleId { value: vehicle }),
                    scope: Some(wire::ScopeId { value: scope }),
                    granted: false,
                    generation: Some(wire::Generation { value: generation }),
                    reason: 0,
                },
            ))];
        }
        let pending = self
            .pending_takeover
            .as_ref()
            .is_some_and(|(v, s)| *v == vehicle && *s == scope);
        if !pending {
            return Vec::new();
        }
        self.pending_takeover = None;
        self.pending_lease = Some((vehicle, scope.clone()));
        vec![ClientAction::SendBootstrap(bootstrap::lease_request(
            vehicle, &scope,
        ))]
    }

    /// Accepts the offer that answers this engine's own pending ask, and
    /// no other.
    fn on_transfer_offered(
        &mut self,
        offered: &wire::ScopeTransferOffered,
        principal_id: u64,
    ) -> Vec<ClientAction> {
        let to = offered.to_principal.as_ref().map(|p| p.value);
        let vehicle = offered.vehicle.as_ref().map_or(0, |v| v.value);
        let scope = offered
            .scope
            .as_ref()
            .map(|s| s.value.clone())
            .unwrap_or_default();
        let pending = self
            .pending_takeover
            .as_ref()
            .is_some_and(|(v, s)| *v == vehicle && *s == scope);
        if to == Some(principal_id) && pending {
            vec![ClientAction::SendBootstrap(bootstrap::transfer_accept(
                vehicle, &scope,
            ))]
        } else {
            Vec::new()
        }
    }

    /// Opens the lane on a commit to this principal; closes it when the
    /// lane's authority went to someone else.
    fn on_transfer_committed(
        &mut self,
        committed: &wire::ScopeTransferCommitted,
        principal_id: u64,
    ) -> Vec<ClientAction> {
        let mut actions = Vec::new();
        let to = committed.to_principal.as_ref().map(|p| p.value);
        let vehicle = committed.vehicle.as_ref().map_or(0, |v| v.value);
        let scope = committed
            .scope
            .as_ref()
            .map(|s| s.value.clone())
            .unwrap_or_default();
        let generation = committed.generation.as_ref().map_or(0, |g| g.value);
        if to == Some(principal_id) {
            if let Some(admission) = self.admission.as_ref() {
                self.pending_takeover = None;
                let mut lane =
                    ControlLane::new(admission.session_id, vehicle, scope.clone(), generation);
                if !self.activation_announced {
                    self.activation_announced = true;
                    actions.push(ClientAction::SendBootstrap(bootstrap::profile_activation(
                        admission.session_id,
                    )));
                }
                lane.bind_profile(
                    bootstrap::NATIVE_PROFILE_REVISION,
                    bootstrap::NATIVE_ACTIVATION_REVISION,
                );
                self.lane = Some(lane);
                actions.push(ClientAction::Emit(ModuleEvent::Lease(
                    wire::LeaseResponse {
                        vehicle: Some(wire::VehicleId { value: vehicle }),
                        scope: Some(wire::ScopeId { value: scope }),
                        granted: true,
                        generation: Some(wire::Generation { value: generation }),
                        reason: 0,
                    },
                )));
            }
        } else if self
            .lane
            .as_ref()
            .is_some_and(|lane| lane.vehicle_id() == vehicle && lane.scope() == scope)
        {
            // Authority moved away from this lane: it is gone, and
            // saying so beats a stream of fenced rejections.
            self.lane = None;
            actions.push(ClientAction::Emit(ModuleEvent::Lease(
                wire::LeaseResponse {
                    vehicle: Some(wire::VehicleId { value: vehicle }),
                    scope: Some(wire::ScopeId { value: scope }),
                    granted: false,
                    generation: Some(wire::Generation { value: generation }),
                    reason: 1,
                },
            )));
        }
        actions
    }
}
