//! Holder-liveness watchdog and link-loss neutralization (ADR-0006, ADR-0008,
//! ADR-0010): a holder that stops sending axis-bearing frames while its
//! connection stays open is released and the vehicle is neutralized, the
//! engagement is never droppable behind the action cap, and straggler frames
//! are fenced. Recovery/activation behavior lives in the `recovery` sibling.

use core::time::Duration;

use pilotage_protocol::{LeaseRequest, SequenceNum};
use pilotage_timing::MonoTimestamp;

use super::{
    VEHICLE, edge_only_frame, engage_trigger, engaged_neutralize, engine_with_silence, frame,
    grant, link_lost, motion, staleness, welcome,
};
use crate::{
    ClientKey, DomainEnvelope, LinkLossTrigger, SessionAction, SessionConfig, SessionEngine,
};

#[test]
fn a_silent_holder_is_released_and_neutralized_after_the_deadline() {
    let mut engine = engine_with_silence(Duration::from_nanos(100));
    let client = ClientKey::new(1);
    welcome(&mut engine, client);
    grant(&mut engine, client, 1); // deadline = 1 + 100 = 101

    // A tick before the deadline neither releases nor neutralizes.
    let early = engine.handle_tick(MonoTimestamp::from_nanos(50));
    assert!(!link_lost(&early), "not silent yet: {:?}", early.actions);
    assert_eq!(engaged_neutralize(&early), 0);

    // A tick at the deadline releases the holder and neutralizes once.
    let fired = engine.handle_tick(MonoTimestamp::from_nanos(101));
    assert!(
        link_lost(&fired),
        "expected HolderLinkLost: {:?}",
        fired.actions
    );
    assert_eq!(
        engaged_neutralize(&fired),
        1,
        "expected exactly one Neutralize: {:?}",
        fired.actions
    );
    assert_eq!(engage_trigger(&fired), Some(LinkLossTrigger::HolderSilence));
}

#[test]
fn an_active_holder_is_never_expired() {
    let mut engine = engine_with_silence(Duration::from_nanos(100));
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    let generation = grant(&mut engine, client, 1); // deadline 101

    // A fresh, accepted frame at t=90 refreshes the deadline to 190.
    let applied = engine.handle_client_message(
        client,
        DomainEnvelope::Frame(frame(
            session,
            generation,
            SequenceNum::new(1),
            MonoTimestamp::from_nanos(90),
        )),
        MonoTimestamp::from_nanos(90),
    );
    assert!(
        applied
            .actions
            .iter()
            .any(|a| matches!(a, SessionAction::ApplyToAdapter { .. })),
        "frame should be accepted: {:?}",
        applied.actions
    );

    // A tick past the ORIGINAL deadline but before the refreshed one does not
    // fire — the holder is actively driving.
    let tick = engine.handle_tick(MonoTimestamp::from_nanos(150));
    assert!(
        !link_lost(&tick),
        "active holder must survive: {:?}",
        tick.actions
    );
    assert_eq!(engaged_neutralize(&tick), 0);
}

#[test]
fn next_deadline_reflects_the_holder_silence_window() {
    let mut engine = engine_with_silence(Duration::from_nanos(100));
    let client = ClientKey::new(1);
    welcome(&mut engine, client);
    grant(&mut engine, client, 1);
    assert_eq!(engine.next_deadline(), Some(MonoTimestamp::from_nanos(101)));
}

#[test]
fn a_disconnect_neutralizes_the_vehicle() {
    // Releasing the lease alone would leave the adapter holding its last
    // command; a disconnect must engage Neutralize, with disconnect
    // provenance on the action.
    let mut engine = engine_with_silence(Duration::from_nanos(100));
    let client = ClientKey::new(1);
    welcome(&mut engine, client);
    grant(&mut engine, client, 1);
    let outcome = engine.handle_client_message(
        client,
        DomainEnvelope::Disconnect,
        MonoTimestamp::from_nanos(5),
    );
    assert_eq!(
        engaged_neutralize(&outcome),
        1,
        "disconnect must neutralize once: {:?}",
        outcome.actions
    );
    assert_eq!(
        engage_trigger(&outcome),
        Some(LinkLossTrigger::HolderDisconnect)
    );
}

#[test]
fn neutralize_is_engaged_exactly_once_per_loss() {
    let mut engine = engine_with_silence(Duration::from_nanos(100));
    let client = ClientKey::new(1);
    welcome(&mut engine, client);
    grant(&mut engine, client, 1); // deadline 101

    let first = engine.handle_tick(MonoTimestamp::from_nanos(101));
    assert_eq!(engaged_neutralize(&first), 1);

    // The holder is gone; a later tick must not re-engage (no re-transmit).
    let second = engine.handle_tick(MonoTimestamp::from_nanos(500));
    assert_eq!(
        engaged_neutralize(&second),
        0,
        "a released holder must not neutralize again: {:?}",
        second.actions
    );
}

#[test]
fn edge_only_frames_do_not_refresh_the_silence_deadline() {
    // Discrete traffic proves the client is alive, not that it is
    // commanding the vehicle: a holder sending only button edges past the
    // silence window is still judged link-lost.
    let mut engine = engine_with_silence(Duration::from_nanos(100));
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    let generation = grant(&mut engine, client, 1); // deadline 101

    let edged = engine.handle_client_message(
        client,
        DomainEnvelope::Frame(edge_only_frame(
            session,
            generation,
            SequenceNum::new(1),
            MonoTimestamp::from_nanos(90),
        )),
        MonoTimestamp::from_nanos(90),
    );
    assert!(
        edged
            .actions
            .iter()
            .any(|a| matches!(a, SessionAction::ApplyToAdapter { .. })),
        "the edge frame itself is accepted: {:?}",
        edged.actions
    );

    // Past the original deadline the holder expires — the edge-only frame
    // did not extend setpoint freshness.
    let tick = engine.handle_tick(MonoTimestamp::from_nanos(150));
    assert!(
        link_lost(&tick),
        "edge-only traffic must not hold the lease open: {:?}",
        tick.actions
    );
    assert_eq!(engaged_neutralize(&tick), 1);
}

#[test]
fn neutralization_is_never_dropped_at_the_action_cap() {
    // A cap of one means the expiry tick's authority broadcast fills the
    // whole per-call budget; the safety lane must still deliver the
    // engagement (fail-closed, never dropped behind the cap).
    let mut engine = SessionEngine::new(
        super::capabilities(),
        staleness(),
        SessionConfig::new(1, "host-test")
            .with_holder_silence(Duration::from_nanos(100))
            .with_max_actions_per_call(1),
    );
    let client = ClientKey::new(1);
    welcome(&mut engine, client);
    // The lease response may be dropped at the cap; the grant itself is
    // engine state, proven by the armed watchdog deadline.
    let granted = engine.handle_client_message(
        client,
        DomainEnvelope::Lease(LeaseRequest {
            vehicle: VEHICLE,
            scope: motion(),
        }),
        MonoTimestamp::from_nanos(1),
    );
    assert!(granted.dropped > 0, "cap of one must drop the response");
    assert_eq!(engine.next_deadline(), Some(MonoTimestamp::from_nanos(101)));

    let fired = engine.handle_tick(MonoTimestamp::from_nanos(101));
    assert_eq!(
        engaged_neutralize(&fired),
        1,
        "the engagement must survive the cap: {:?}",
        fired.actions
    );
}

#[test]
fn a_straggler_frame_after_expiry_is_fenced_not_applied() {
    let mut engine = engine_with_silence(Duration::from_nanos(100));
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    let generation = grant(&mut engine, client, 1); // deadline 101

    let _released = engine.handle_tick(MonoTimestamp::from_nanos(101)); // advances generation

    // A late frame at the pre-loss generation cannot drive the vehicle.
    let late = engine.handle_client_message(
        client,
        DomainEnvelope::Frame(frame(
            session,
            generation,
            SequenceNum::new(9),
            MonoTimestamp::from_nanos(120),
        )),
        MonoTimestamp::from_nanos(120),
    );
    assert!(
        !late
            .actions
            .iter()
            .any(|a| matches!(a, SessionAction::ApplyToAdapter { .. })),
        "a straggler after link loss must be fenced: {:?}",
        late.actions
    );
}
