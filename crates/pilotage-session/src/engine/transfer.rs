//! Cooperative handover: ask, offer, accept (CLIENT-09).
//!
//! The authority engine owns every state change; these handlers validate
//! the sender and route. The ask changes nothing and is broadcast so the
//! holder's operator can decide; the offer and the accept are the
//! existing two-phase handover, driven from the wire for the first time.

use pilotage_authority::AuthorityCommand;
use pilotage_timing::MonoTimestamp;

use super::{Actions, LinkLossTrigger, SessionEngine};
use crate::message::ClientKey;

/// How long an offer stands before it expires back to the holder. Long
/// enough to read a banner and press a button; short enough that a
/// forgotten offer does not stand all flight.
const OFFER_TTL: core::time::Duration = core::time::Duration::from_secs(30);

impl SessionEngine {
    /// A non-holder asks for a scope: validated, then broadcast. The
    /// authority engine refuses an ask from the holder itself or for an
    /// unassigned scope, and the rejection effect goes back as the same
    /// broadcastable vocabulary everything else uses.
    pub(super) fn on_transfer_request(
        &mut self,
        client: ClientKey,
        request: pilotage_protocol::ScopeTransferRequest,
        now: MonoTimestamp,
        actions: &mut Actions,
    ) {
        let Some(principal) = self.welcomed_principal(client, actions) else {
            return;
        };
        let Some(pair) = self.authority_pair(request.vehicle, &request.scope) else {
            return;
        };
        let effects = self.authority.handle(
            AuthorityCommand::RequestTransfer {
                vehicle: pair.0,
                scope: pair.1,
                from: principal,
            },
            now,
        );
        self.fan_out_authority(effects, now, LinkLossTrigger::AuthorityRevoked, actions);
    }

    /// The holder offers its scope. The authority engine enforces that
    /// the sender holds it; a stranger's offer is rejected there.
    pub(super) fn on_transfer_offer(
        &mut self,
        client: ClientKey,
        offer: pilotage_protocol::ScopeTransferOffer,
        now: MonoTimestamp,
        actions: &mut Actions,
    ) {
        let Some(principal) = self.welcomed_principal(client, actions) else {
            return;
        };
        let Some(pair) = self.authority_pair(offer.vehicle, &offer.scope) else {
            return;
        };
        let effects = self.authority.handle(
            AuthorityCommand::Offer {
                vehicle: pair.0,
                scope: pair.1,
                from: principal,
                to: offer.to,
                ttl: OFFER_TTL,
            },
            now,
        );
        self.fan_out_authority(effects, now, LinkLossTrigger::AuthorityRevoked, actions);
    }

    /// The offered principal accepts; the commit fences a new generation
    /// and the fan-out moves the holder records with it.
    pub(super) fn on_transfer_accept(
        &mut self,
        client: ClientKey,
        accept: pilotage_protocol::ScopeTransferAccept,
        now: MonoTimestamp,
        actions: &mut Actions,
    ) {
        let Some(principal) = self.welcomed_principal(client, actions) else {
            return;
        };
        let Some(pair) = self.authority_pair(accept.vehicle, &accept.scope) else {
            return;
        };
        let expected_generation = self
            .clients
            .generation_of(&pair)
            .unwrap_or_else(|| pilotage_protocol::Generation::new(0));
        let effects = self.authority.handle(
            AuthorityCommand::Accept {
                vehicle: pair.0,
                scope: pair.1.clone(),
                by: principal,
                expected_generation,
            },
            now,
        );
        let committed = effects.iter().any(|effect| {
            matches!(
                effect,
                pilotage_authority::AuthorityEffect::ScopeTransferCommitted { .. }
            )
        });
        self.fan_out_authority(effects, now, LinkLossTrigger::AuthorityRevoked, actions);
        if committed {
            // The new holder commands the member scope it accepted, the
            // same binding a grant records.
            self.clients.set_held_member(pair, accept.scope);
        }
    }
}
