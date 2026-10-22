//! Authority-event handling for the mission principal (ADR-0010,
//! ADR-0025, ADR-0051): a human who asks for the motion scope receives an
//! offer; a transfer, revocation, or override away from this principal
//! holds the mission; an offer back is accepted, and the mission resumes
//! on the committed transfer. Only a closed connection fences the task
//! for good.

use pilotage_protocol::{
    Generation, PrincipalId, ScopeId, ScopeTransferAccept, ScopeTransferOffer, wire,
};
use pilotage_session::DomainEnvelope;
use tracing::{info, warn};

use crate::runtime::HOST_VEHICLE;

use super::{MISSION_SCOPE, MissionTask};

impl MissionTask {
    /// Handles one authority event. Returns the reply the event asks
    /// for: an offer to a human who requested the scope, or the
    /// acceptance of an offer back to this principal.
    pub(super) fn on_authority(&mut self, bytes: &[u8]) -> Option<DomainEnvelope> {
        let envelope = match pilotage_protocol::decode_envelope_length_delimited(bytes) {
            Ok((envelope, _rest)) => envelope,
            Err(error) => {
                warn!(%error, "undecodable authority event");
                return None;
            }
        };
        let Some(wire::envelope::Payload::AuthorityEvent(event)) = envelope.payload else {
            return None;
        };
        use wire::authority_event::Event;
        match event.event {
            Some(Event::ScopeLeaseGranted(granted)) => self.on_granted_event(&granted),
            Some(Event::ScopeLeaseRevoked(revoked)) => self.on_revoked_event(&revoked),
            Some(Event::EmergencyOverrideApplied(applied)) => self.on_override_event(&applied),
            Some(Event::ScopeTransferCommitted(committed)) => self.on_transfer_event(&committed),
            Some(Event::ScopeTransferRequested(requested)) => {
                return self.on_transfer_requested(&requested);
            }
            Some(Event::ScopeTransferOffered(offered)) => {
                return self.on_transfer_offered(&offered);
            }
            _ => {}
        }
        None
    }

    fn on_granted_event(&mut self, granted: &wire::ScopeLeaseGranted) {
        if !our_pair(granted.vehicle.as_ref(), granted.scope.as_ref()) || self.fenced {
            return;
        }
        if self.our_principal(granted.principal.as_ref()) {
            if let Some(generation) = granted.generation.as_ref() {
                self.take_generation(Generation::new(generation.value));
            }
        } else if self.generation.is_some() {
            self.hold("the motion scope was granted to another principal");
        }
    }

    fn on_revoked_event(&mut self, revoked: &wire::ScopeLeaseRevoked) {
        if !our_pair(revoked.vehicle.as_ref(), revoked.scope.as_ref()) {
            return;
        }
        if self.our_principal(revoked.principal.as_ref()) && self.generation.is_some() {
            self.hold("the motion lease was revoked");
        }
    }

    fn on_override_event(&mut self, applied: &wire::EmergencyOverrideApplied) {
        if !our_pair(applied.vehicle.as_ref(), applied.scope.as_ref()) {
            return;
        }
        if !self.our_principal(applied.principal.as_ref()) && self.generation.is_some() {
            self.hold("an emergency override preempted the motion scope");
        }
    }

    /// A committed transfer either takes the scope away, which holds the
    /// mission, or brings it back under a new generation, which resumes.
    fn on_transfer_event(&mut self, committed: &wire::ScopeTransferCommitted) {
        if !our_pair(committed.vehicle.as_ref(), committed.scope.as_ref()) || self.fenced {
            return;
        }
        if self.our_principal(committed.to_principal.as_ref()) {
            if let Some(generation) = committed.generation.as_ref() {
                self.take_generation(Generation::new(generation.value));
            }
        } else if self.generation.is_some() {
            self.hold("the motion scope transferred to another principal");
        }
    }

    /// A human asked for the scope this principal holds: offer it. Human
    /// takeover wins (ADR-0025); the mission holds on the commit.
    fn on_transfer_requested(
        &mut self,
        requested: &wire::ScopeTransferRequested,
    ) -> Option<DomainEnvelope> {
        if !our_pair(requested.vehicle.as_ref(), requested.scope.as_ref())
            || self.fenced
            || self.generation.is_none()
        {
            return None;
        }
        let requester = requested.from_principal.as_ref()?;
        if self.our_principal(Some(requester)) {
            return None;
        }
        info!(
            requester = requester.value,
            "a principal requested the motion scope; offering it"
        );
        self.update(|status| status.offers = status.offers.wrapping_add(1));
        Some(DomainEnvelope::TransferOffer(ScopeTransferOffer {
            vehicle: HOST_VEHICLE,
            scope: ScopeId::new(MISSION_SCOPE),
            to: PrincipalId::new(requester.value),
        }))
    }

    /// A holder offered the scope back to this principal: accept. The
    /// resume follows the committed transfer, never the offer.
    fn on_transfer_offered(
        &mut self,
        offered: &wire::ScopeTransferOffered,
    ) -> Option<DomainEnvelope> {
        if !our_pair(offered.vehicle.as_ref(), offered.scope.as_ref())
            || self.fenced
            || !self.our_principal(offered.to_principal.as_ref())
        {
            return None;
        }
        info!(
            from = offered
                .from_principal
                .as_ref()
                .map(|principal| principal.value),
            "the motion scope was offered back; accepting"
        );
        Some(DomainEnvelope::TransferAccept(ScopeTransferAccept {
            vehicle: HOST_VEHICLE,
            scope: ScopeId::new(MISSION_SCOPE),
        }))
    }

    fn our_principal(&self, principal: Option<&wire::PrincipalId>) -> bool {
        match (self.principal, principal) {
            (Some(ours), Some(event)) => event.value == ours.as_u64(),
            _ => false,
        }
    }

    /// Holds a generation this principal now frames against. A held
    /// mission resumes: the first frame under the new generation is
    /// neutral, which a vehicle that requires neutral input on a new
    /// lease admits.
    fn take_generation(&mut self, generation: Generation) {
        self.generation = Some(generation);
        if self.held {
            self.held = false;
            self.neutral_frames = 1;
            let now = self.mission_now();
            let held = self
                .mission
                .as_mut()
                .and_then(|mission| mission.resume(now));
            info!(
                generation = generation.as_u64(),
                held_ns = held.map(|held| held.as_nanos()),
                "mission resumed under a new generation"
            );
            self.update(|status| status.resumes = status.resumes.wrapping_add(1));
        }
        self.update(|status| {
            status.held = false;
            status.lease_generation = Some(generation.as_u64());
        });
    }

    /// Holds the mission: a human took the motion scope. The engine keeps
    /// its ownship fresh and pauses its sequencing clock; this task stops
    /// framing until a transfer brings the scope back (ADR-0051).
    pub(super) fn hold(&mut self, reason: &str) {
        if self.fenced || self.held {
            return;
        }
        self.held = true;
        self.generation = None;
        let now = self.mission_now();
        if let Some(mission) = self.mission.as_mut() {
            mission.hold(now);
        }
        warn!(
            reason,
            "mission held; waiting for the motion scope to come back"
        );
        self.update(|status| {
            status.held = true;
            status.lease_generation = None;
        });
    }

    /// Drops the held generation and stops framing, permanently for this
    /// task. Only the engine closing the connection ends a mission this
    /// way; every authority transition is a hold.
    pub(super) fn fence(&mut self, reason: &str) {
        if self.fenced {
            return;
        }
        self.fenced = true;
        self.held = false;
        self.generation = None;
        warn!(reason, "mission authority fenced; holding without re-lease");
        self.update(|status| {
            status.fenced = true;
            status.held = false;
            status.lease_generation = None;
        });
    }
}

/// Whether an authority event names this principal's (vehicle, scope).
fn our_pair(vehicle: Option<&wire::VehicleId>, scope: Option<&wire::ScopeId>) -> bool {
    vehicle.is_some_and(|id| id.value == HOST_VEHICLE.as_u64())
        && scope.is_some_and(|id| id.value == MISSION_SCOPE)
}

#[cfg(test)]
mod tests;
