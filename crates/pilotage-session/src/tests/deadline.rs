//! Deadline pass-through: `next_deadline` and `handle_tick` delegate to the
//! embedded authority engine.
//!
//! The increment-0 [`DomainEnvelope`] set carries no offer/accept handover
//! messages (ADR-0010 two-phase handover is a later increment), so no offer is
//! ever pending here: `next_deadline` reflects the authority engine's own
//! `None`, and a tick produces no actions. These tests pin the delegation so a
//! future increment that adds offers inherits correct scheduling behavior.
//!
//! [`DomainEnvelope`]: crate::DomainEnvelope

use pilotage_timing::MonoTimestamp;

use super::{VEHICLE, engine, motion, welcome};
use crate::{ClientKey, DomainEnvelope};

#[test]
fn no_pending_offer_means_no_deadline() {
    let engine = engine();
    assert_eq!(engine.next_deadline(), None);
}

#[test]
fn deadline_stays_none_after_a_grant() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    welcome(&mut engine, client);
    let _granted = engine.handle_client_message(
        client,
        DomainEnvelope::Lease(pilotage_protocol::LeaseRequest {
            vehicle: VEHICLE,
            scope: motion(),
        }),
        MonoTimestamp::from_nanos(1),
    );
    // A direct grant does not open an offer, so there is nothing to expire.
    assert_eq!(engine.next_deadline(), None);
}

#[test]
fn tick_without_pending_offers_yields_no_actions() {
    let mut engine = engine();
    let outcome = engine.handle_tick(MonoTimestamp::from_nanos(1_000_000));
    assert!(outcome.actions.is_empty());
    assert_eq!(outcome.dropped, 0);
}
