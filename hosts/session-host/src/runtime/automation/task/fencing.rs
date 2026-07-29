//! Authority-event fencing for the mission principal: any transition
//! that moves the motion scope away from this principal stops the
//! mission cold, and re-leasing is a human decision, never automation's
//! (ADR-0025).

use pilotage_protocol::{Generation, wire};
use tracing::warn;

use crate::runtime::HOST_VEHICLE;

use super::{MISSION_SCOPE, MissionTask};

impl MissionTask {
    pub(super) fn on_authority(&mut self, bytes: &[u8]) {
        let envelope = match pilotage_protocol::decode_envelope_length_delimited(bytes) {
            Ok((envelope, _rest)) => envelope,
            Err(error) => {
                warn!(%error, "undecodable authority event");
                return;
            }
        };
        let Some(wire::envelope::Payload::AuthorityEvent(event)) = envelope.payload else {
            return;
        };
        use wire::authority_event::Event;
        match event.event {
            Some(Event::ScopeLeaseGranted(granted)) => self.on_granted_event(&granted),
            Some(Event::ScopeLeaseRevoked(revoked)) => self.on_revoked_event(&revoked),
            Some(Event::EmergencyOverrideApplied(applied)) => self.on_override_event(&applied),
            Some(Event::ScopeTransferCommitted(committed)) => self.on_transfer_event(&committed),
            _ => {}
        }
    }

    fn on_granted_event(&mut self, granted: &wire::ScopeLeaseGranted) {
        if !our_pair(granted.vehicle.as_ref(), granted.scope.as_ref()) {
            return;
        }
        if self.our_principal(granted.principal.as_ref()) {
            if let Some(generation) = granted.generation.as_ref() {
                self.generation = Some(Generation::new(generation.value));
                self.update(|status| status.lease_generation = Some(generation.value));
            }
        } else if self.generation.is_some() {
            self.fence("the motion scope was granted to another principal");
        }
    }

    fn on_revoked_event(&mut self, revoked: &wire::ScopeLeaseRevoked) {
        if !our_pair(revoked.vehicle.as_ref(), revoked.scope.as_ref()) {
            return;
        }
        if self.our_principal(revoked.principal.as_ref()) && self.generation.is_some() {
            self.fence("the motion lease was revoked");
        }
    }

    fn on_override_event(&mut self, applied: &wire::EmergencyOverrideApplied) {
        if !our_pair(applied.vehicle.as_ref(), applied.scope.as_ref()) {
            return;
        }
        if !self.our_principal(applied.principal.as_ref()) && self.generation.is_some() {
            self.fence("an emergency override preempted the motion scope");
        }
    }

    fn on_transfer_event(&mut self, committed: &wire::ScopeTransferCommitted) {
        if !our_pair(committed.vehicle.as_ref(), committed.scope.as_ref()) {
            return;
        }
        if !self.our_principal(committed.to_principal.as_ref()) && self.generation.is_some() {
            self.fence("the motion scope transferred to another principal");
        }
    }

    fn our_principal(&self, principal: Option<&wire::PrincipalId>) -> bool {
        match (self.principal, principal) {
            (Some(ours), Some(event)) => event.value == ours.as_u64(),
            _ => false,
        }
    }

    /// Drops the held generation and stops framing, permanently for this
    /// task: re-leasing is a human decision, never automation's
    /// (ADR-0025).
    pub(super) fn fence(&mut self, reason: &str) {
        if self.fenced {
            return;
        }
        self.fenced = true;
        self.generation = None;
        warn!(reason, "mission authority fenced; holding without re-lease");
        self.update(|status| {
            status.fenced = true;
            status.lease_generation = None;
        });
    }
}

/// Whether an authority event names this principal's (vehicle, scope).
fn our_pair(vehicle: Option<&wire::VehicleId>, scope: Option<&wire::ScopeId>) -> bool {
    vehicle.is_some_and(|id| id.value == HOST_VEHICLE.as_u64())
        && scope.is_some_and(|id| id.value == MISSION_SCOPE)
}
