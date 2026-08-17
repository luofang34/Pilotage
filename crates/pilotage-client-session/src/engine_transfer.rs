//! The engine's half of a cooperative handover: accepting the offer
//! that answers its own ask, opening the lane on the commit, closing it
//! when authority departs, and falling back to a plain lease when the
//! scope frees up mid-ask.

use pilotage_protocol::wire;

use crate::action::{ClientAction, ModuleEvent};
use crate::bootstrap;
use crate::engine::{ClientEngine, Escalation};

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
        if self.lanes.remove(&(vehicle, scope.clone())).is_some() {
            let generation = revoked.generation.as_ref().map_or(0, |g| g.value);
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
        self.pending_leases
            .insert((vehicle, scope.clone()), Escalation::Cooperative);
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
            if self.admission.is_some() {
                self.pending_takeover = None;
                actions = self.open_lane(vehicle, scope.clone(), generation);
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
        } else if self.lanes.remove(&(vehicle, scope.clone())).is_some() {
            // Authority moved away from this lane: it is gone, and
            // saying so beats a stream of fenced rejections.
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
