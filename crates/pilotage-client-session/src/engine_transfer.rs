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
            // An emergency override by someone else takes the scope from
            // this engine's lanes, as a committed transfer does.
            Some(wire::authority_event::Event::EmergencyOverrideApplied(applied))
                if applied.principal.as_ref().map(|p| p.value) != Some(principal_id) =>
            {
                let vehicle = applied.vehicle.as_ref().map_or(0, |v| v.value);
                let scope = applied
                    .scope
                    .as_ref()
                    .map(|s| s.value.clone())
                    .unwrap_or_default();
                let generation = applied.generation.as_ref().map_or(0, |g| g.value);
                self.end_group_lanes(vehicle, &scope, generation, 1)
                    .unwrap_or_default()
            }
            // The offer's time ran out and the scope stayed where it
            // was: nothing is pending any more, so a later ask is a
            // fresh one rather than a wait for an answer that will
            // never come.
            Some(wire::authority_event::Event::ScopeTransferExpired(expired)) => {
                let vehicle = expired.vehicle.as_ref().map_or(0, |v| v.value);
                let scope = expired
                    .scope
                    .as_ref()
                    .map(|s| s.value.clone())
                    .unwrap_or_default();
                self.clear_pending_takeover(vehicle, &scope);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// A revocation lands two ways. A revocation of this engine's own
    /// lane, or of the group it is held under, closes the lane and says
    /// so: an operator must never work a control surface the host already
    /// took back. A stranger's revocation while this engine's ask is pending
    /// frees the scope, so the ask becomes an ordinary lease request.
    fn on_scope_freed(&mut self, revoked: &wire::ScopeLeaseRevoked) -> Vec<ClientAction> {
        let vehicle = revoked.vehicle.as_ref().map_or(0, |v| v.value);
        let scope = revoked
            .scope
            .as_ref()
            .map(|s| s.value.clone())
            .unwrap_or_default();
        let generation = revoked.generation.as_ref().map_or(0, |g| g.value);
        if let Some(actions) = self.end_group_lanes(vehicle, &scope, generation, 0) {
            return actions;
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

    /// Ends every lane of `vehicle` held under authority `group` (the group
    /// itself or one of its members) that the event at `generation`
    /// supersedes, reporting each as a denied lease with `reason`.
    ///
    /// Authority events name the group, so revoking or handing over the
    /// flight group ends the direct lane too. Fenced by generation: a
    /// lease-ending event advances the generation past the lane it ends,
    /// and every later grant advances it again, so an event at or below a
    /// lane's generation ended an earlier lease. Authority events and lease
    /// replies ride different streams, and a scheme switch's release can
    /// arrive after the grant that followed it; that late event must not
    /// close the new lane.
    ///
    /// `None` when the engine holds no lane of the group at all; an empty
    /// list when every such lane outlived the event.
    fn end_group_lanes(
        &mut self,
        vehicle: u64,
        group: &str,
        generation: u64,
        reason: i32,
    ) -> Option<Vec<ClientAction>> {
        let members: Vec<((u64, String), u64)> = self
            .lanes
            .iter()
            .filter(|((lane_vehicle, lane_scope), _)| {
                *lane_vehicle == vehicle && self.in_group(vehicle, lane_scope, group)
            })
            .map(|(key, lane)| (key.clone(), lane.generation()))
            .collect();
        if members.is_empty() {
            return None;
        }
        let mut actions = Vec::new();
        for (key, lane_generation) in members {
            if generation <= lane_generation || self.lanes.remove(&key).is_none() {
                continue;
            }
            let (vehicle, scope) = key;
            actions.push(ClientAction::Emit(ModuleEvent::Lease(
                wire::LeaseResponse {
                    vehicle: Some(wire::VehicleId { value: vehicle }),
                    scope: Some(wire::ScopeId { value: scope }),
                    granted: false,
                    generation: Some(wire::Generation { value: generation }),
                    reason,
                },
            )));
        }
        Some(actions)
    }

    /// Whether `lane_scope` is `group` or a member of it.
    fn in_group(&self, vehicle: u64, lane_scope: &str, group: &str) -> bool {
        lane_scope == group
            || self
                .admission
                .as_ref()
                .is_some_and(|admission| admission.authority_group_of(vehicle, lane_scope) == group)
    }

    /// Drops a pending ask this scope's outcome has settled. An ask
    /// that outlives the handover it belongs to is a shell told to keep
    /// waiting for an answer that can no longer come.
    fn clear_pending_takeover(&mut self, vehicle: u64, scope: &str) {
        if self
            .pending_takeover
            .as_ref()
            .is_some_and(|(pending_vehicle, pending_scope)| {
                *pending_vehicle == vehicle && pending_scope == scope
            })
        {
            self.pending_takeover = None;
        }
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
            let requested = self
                .pending_takeover
                .as_ref()
                .is_some_and(|(v, s)| *v == vehicle && *s == scope);
            if self.admission.is_some() && requested {
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
        } else {
            // The scope went to someone else. Whatever this engine was
            // waiting for is settled either way, and only a lane it
            // actually held is worth reporting as lost.
            self.clear_pending_takeover(vehicle, &scope);
            actions.extend(
                self.end_group_lanes(vehicle, &scope, generation, 1)
                    .unwrap_or_default(),
            );
        }
        actions
    }
}
