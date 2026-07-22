//! Frames: accept a fresh in-generation frame, fence a stale generation,
//! reject after release, reject an unknown scope, and reject a stale-age frame.

use pilotage_protocol::{
    FrameRejectionReason, Generation, ScopeId, ScopedControlFrame, SequenceNum,
};
use pilotage_timing::MonoTimestamp;

use super::{VEHICLE, engine, frame, motion, welcome};
use crate::{ClientKey, DomainEnvelope, SessionAction, SessionEngine};

/// Welcomes `client`, leases the motion scope to it, and returns its session
/// id (the scope is now `Held` at generation 1).
fn welcomed_holder(engine: &mut SessionEngine, client: ClientKey) -> pilotage_protocol::SessionId {
    let session = welcome(engine, client);
    let _granted = engine.handle_client_message(
        client,
        DomainEnvelope::Lease(pilotage_protocol::LeaseRequest {
            vehicle: VEHICLE,
            scope: motion(),
        }),
        MonoTimestamp::from_nanos(1),
    );
    session
}

fn submit(
    engine: &mut SessionEngine,
    client: ClientKey,
    frame: ScopedControlFrame,
    now: MonoTimestamp,
) -> Vec<SessionAction> {
    engine
        .handle_client_message(client, DomainEnvelope::Frame(frame), now)
        .actions
}

#[test]
fn in_generation_fresh_frame_is_applied_to_adapter() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    let session = welcomed_holder(&mut engine, client);
    let frame = frame(
        session,
        Generation::new(1),
        SequenceNum::new(0),
        MonoTimestamp::from_nanos(1_000),
    );
    let actions = submit(&mut engine, client, frame, MonoTimestamp::from_nanos(1_010));
    assert!(matches!(
        actions.as_slice(),
        [SessionAction::ApplyToAdapter { .. }]
    ));
}

#[test]
fn wrong_generation_frame_is_fenced() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    let session = welcomed_holder(&mut engine, client);
    // The holder is at generation 1; a frame stamped generation 0 is stale.
    let frame = frame(
        session,
        Generation::new(0),
        SequenceNum::new(1),
        MonoTimestamp::from_nanos(1_000),
    );
    let actions = submit(&mut engine, client, frame, MonoTimestamp::from_nanos(1_010));
    match actions.as_slice() {
        [SessionAction::RejectFrame { rejection, .. }] => {
            assert_eq!(rejection.reason, FrameRejectionReason::StaleGeneration);
            assert_eq!(rejection.current_generation.as_u64(), 1);
        }
        other => panic!("expected a stale-generation rejection, got {other:?}"),
    }
}

#[test]
fn frame_after_holder_release_is_rejected_no_holder() {
    let mut engine = engine();
    let holder = ClientKey::new(1);
    let observer = ClientKey::new(2);
    let _session = welcomed_holder(&mut engine, holder);
    // A second client sends the late frame so the engine still knows it (the
    // holder itself is gone after disconnect). Both frames target the same
    // scope, now unassigned with an advanced generation. The observer sends
    // under ITS OWN session — a foreign session id would be refused before
    // the holder check.
    let observer_session = welcome(&mut engine, observer);
    let _released = engine.handle_client_message(
        holder,
        DomainEnvelope::Disconnect,
        MonoTimestamp::from_nanos(2_000),
    );
    let frame = frame(
        observer_session,
        Generation::new(2),
        SequenceNum::new(0),
        MonoTimestamp::from_nanos(2_000),
    );
    let actions = submit(
        &mut engine,
        observer,
        frame,
        MonoTimestamp::from_nanos(2_010),
    );
    match actions.as_slice() {
        [SessionAction::RejectFrame { rejection, .. }] => {
            assert_eq!(rejection.reason, FrameRejectionReason::NoHolder);
        }
        other => panic!("expected a no-holder rejection, got {other:?}"),
    }
}

#[test]
fn non_holder_in_generation_frame_is_fenced() {
    let mut engine = engine();
    let holder = ClientKey::new(1);
    let forger = ClientKey::new(2);
    let _session = welcomed_holder(&mut engine, holder);
    // The forger completed the handshake but never leased the scope. It knows
    // the current generation (grants are broadcast) and forges an
    // in-generation frame UNDER ITS OWN SESSION (a foreign session id would
    // be refused earlier). Fencing on identity must reject it.
    let forger_session = welcome(&mut engine, forger);
    let frame = frame(
        forger_session,
        Generation::new(1),
        SequenceNum::new(0),
        MonoTimestamp::from_nanos(1_000),
    );
    let actions = submit(&mut engine, forger, frame, MonoTimestamp::from_nanos(1_010));
    match actions.as_slice() {
        [SessionAction::RejectFrame { rejection, .. }] => {
            assert_eq!(rejection.reason, FrameRejectionReason::NoHolder);
        }
        other => panic!("expected a no-holder rejection for a non-holder, got {other:?}"),
    }
}

#[test]
fn unknown_scope_frame_is_rejected() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    let mut frame = frame(
        session,
        Generation::new(0),
        SequenceNum::new(0),
        MonoTimestamp::from_nanos(0),
    );
    frame.scope = ScopeId::new("vehicle.ghost");
    let actions = submit(&mut engine, client, frame, MonoTimestamp::from_nanos(10));
    match actions.as_slice() {
        [SessionAction::RejectFrame { rejection, .. }] => {
            assert_eq!(rejection.reason, FrameRejectionReason::UnknownScope);
        }
        other => panic!("expected an unknown-scope rejection, got {other:?}"),
    }
}

#[test]
fn stale_age_frame_is_rejected_too_old() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    let session = welcomed_holder(&mut engine, client);
    // The client's sample clock shares no epoch with the host clock, so a
    // FIRST frame establishes the correlation floor and reads fresh...
    let fresh = frame(
        session,
        Generation::new(1),
        SequenceNum::new(1),
        MonoTimestamp::from_nanos(1_000),
    );
    let accepted = submit(
        &mut engine,
        client,
        fresh,
        MonoTimestamp::from_nanos(11_000),
    );
    assert!(
        matches!(accepted.as_slice(), [SessionAction::ApplyToAdapter { .. }]),
        "the floor-defining frame is fresh by definition: {accepted:?}"
    );
    // ...and a later frame 60 ms PAST that floor (against a 50 ms policy)
    // is stale on CORRELATED age, not raw cross-clock subtraction.
    let stale = frame(
        session,
        Generation::new(1),
        SequenceNum::new(2),
        MonoTimestamp::from_nanos(2_000),
    );
    let now = MonoTimestamp::from_nanos(60_000_000 + 12_000);
    let actions = submit(&mut engine, client, stale, now);
    match actions.as_slice() {
        [SessionAction::RejectFrame { rejection, .. }] => {
            assert_eq!(rejection.reason, FrameRejectionReason::TooOld);
        }
        other => panic!("expected a too-old rejection, got {other:?}"),
    }
}
