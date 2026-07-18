//! Deadline scheduling: `next_deadline` folds the authority engine's offer
//! expiry together with the holder-silence watchdog.
//!
//! The increment-0 [`DomainEnvelope`] set carries no offer/accept handover
//! messages (ADR-0010 two-phase handover is a later increment), so no offer is
//! ever pending here; the only deadline a grant creates is the holder-silence
//! window. These tests pin that scheduling so a future increment that adds
//! offers inherits correct behavior.
//!
//! [`DomainEnvelope`]: crate::DomainEnvelope

use pilotage_timing::MonoTimestamp;

use super::{VEHICLE, engine, motion, welcome};
use crate::{ClientKey, DomainEnvelope, SessionConfig};

#[test]
fn no_holder_and_no_offer_means_no_deadline() {
    let engine = engine();
    assert_eq!(engine.next_deadline(), None);
}

#[test]
fn deadline_after_a_grant_is_the_holder_silence_window() {
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
    // A direct grant opens no handover offer, but it starts the holder-silence
    // watchdog, so the next deadline is the grant time plus the silence window.
    assert_eq!(
        engine.next_deadline(),
        Some(MonoTimestamp::from_nanos(1).saturating_add(SessionConfig::DEFAULT_HOLDER_SILENCE)),
    );
}

#[test]
fn tick_without_pending_offers_yields_no_actions() {
    let mut engine = engine();
    let outcome = engine.handle_tick(MonoTimestamp::from_nanos(1_000_000));
    assert!(outcome.actions.is_empty());
    assert_eq!(outcome.dropped, 0);
}
