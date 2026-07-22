//! Datagram ingress hardening (CTRL-01): session attribution, wrap-aware
//! sequence ordering judged ahead of liveness refresh and recovery clearing,
//! correlated staleness instead of raw cross-clock subtraction, the
//! typed-only production default, and deadline expiry judged BEFORE any
//! incoming message can refresh it.

use core::time::Duration;

use pilotage_protocol::{FrameRejectionReason, LeaseRelease, LeaseRequest, SequenceNum, SessionId};
use pilotage_timing::MonoTimestamp;

use super::{
    VEHICLE, engaged_neutralize, engine, engine_with_silence, frame, grant, link_lost, motion,
    neutral_frame, staleness, welcome,
};
use crate::{
    ClientKey, DomainEnvelope, SessionAction, SessionConfig, SessionEngine, SessionOutcome,
};

fn rejection_reason(outcome: &SessionOutcome) -> Option<FrameRejectionReason> {
    outcome.actions.iter().find_map(|action| match action {
        SessionAction::RejectFrame { rejection, .. } => Some(rejection.reason),
        _ => None,
    })
}

fn applied(outcome: &SessionOutcome) -> bool {
    outcome
        .actions
        .iter()
        .any(|action| matches!(action, SessionAction::ApplyToAdapter { .. }))
}

fn submit(
    engine: &mut SessionEngine,
    client: ClientKey,
    frame: pilotage_protocol::ScopedControlFrame,
    now: u64,
) -> SessionOutcome {
    engine.handle_client_message(
        client,
        DomainEnvelope::Frame(frame),
        MonoTimestamp::from_nanos(now),
    )
}

/// A host built from the DEFAULT config admits no legacy numeric payloads:
/// they bypass profile-activation binding and translate button edges into
/// uncorrelated actions, so they exist only under the explicit SIMULATION
/// compatibility mode.
#[test]
fn typed_only_is_the_production_default() {
    let mut engine = SessionEngine::new(
        super::capabilities(),
        staleness(),
        SessionConfig::new(1, "host-test"),
    );
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    let generation = grant(&mut engine, client, 1);
    let payload_frame = frame(
        session,
        generation,
        SequenceNum::new(1),
        MonoTimestamp::from_nanos(5),
    );
    let outcome = submit(&mut engine, client, payload_frame, 10);
    assert_eq!(
        rejection_reason(&outcome),
        Some(FrameRejectionReason::LegacyDisabled),
        "a numeric payload frame must be refused by default: {:?}",
        outcome.actions
    );
    assert!(!applied(&outcome));
}

/// A frame naming a session other than the sender's own cannot be attributed
/// to the sender's records (activation binding, sequence state) and is
/// refused before anything else reads it — even when generation and holder
/// identity would otherwise pass.
#[test]
fn a_frame_naming_a_foreign_session_is_rejected() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    let generation = grant(&mut engine, client, 1);
    let mut forged = frame(
        session,
        generation,
        SequenceNum::new(1),
        MonoTimestamp::from_nanos(5),
    );
    forged.session = SessionId::new(session.as_u64().wrapping_add(999));
    let outcome = submit(&mut engine, client, forged, 10);
    assert_eq!(
        rejection_reason(&outcome),
        Some(FrameRejectionReason::SessionMismatch),
        "got {:?}",
        outcome.actions
    );
    assert!(!applied(&outcome));
}

/// A duplicated datagram is refused BEFORE it can refresh liveness: the
/// holder-silence deadline must stay where the last ADMITTED frame put it,
/// or a replaying sender could hold a dead lease open.
#[test]
fn a_duplicate_sequence_is_rejected_and_does_not_refresh_liveness() {
    let mut engine = engine_with_silence(Duration::from_nanos(100));
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    let generation = grant(&mut engine, client, 1);

    let first = frame(
        session,
        generation,
        SequenceNum::new(1),
        MonoTimestamp::from_nanos(40),
    );
    assert!(applied(&submit(&mut engine, client, first.clone(), 40)));
    assert_eq!(
        engine.next_deadline(),
        Some(MonoTimestamp::from_nanos(140)),
        "the admitted frame refreshes the deadline"
    );

    let outcome = submit(&mut engine, client, first, 90);
    assert_eq!(
        rejection_reason(&outcome),
        Some(FrameRejectionReason::StaleSequence),
        "got {:?}",
        outcome.actions
    );
    assert_eq!(
        engine.next_deadline(),
        Some(MonoTimestamp::from_nanos(140)),
        "the duplicate must not push the deadline"
    );
}

/// A reordered datagram (sequence behind the newest admitted one) is refused:
/// within one generation the sequence must strictly advance.
#[test]
fn an_out_of_order_sequence_is_rejected() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    let generation = grant(&mut engine, client, 1);
    let newest = frame(
        session,
        generation,
        SequenceNum::new(5),
        MonoTimestamp::from_nanos(5),
    );
    assert!(applied(&submit(&mut engine, client, newest, 10)));
    let straggler = frame(
        session,
        generation,
        SequenceNum::new(3),
        MonoTimestamp::from_nanos(6),
    );
    let outcome = submit(&mut engine, client, straggler, 11);
    assert_eq!(
        rejection_reason(&outcome),
        Some(FrameRejectionReason::StaleSequence),
        "got {:?}",
        outcome.actions
    );
}

/// Ordering is wrap-aware: `u32::MAX -> 0` is a forward step, not a replay.
#[test]
fn a_sequence_wrap_forward_is_admitted() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    let generation = grant(&mut engine, client, 1);
    let at_max = frame(
        session,
        generation,
        SequenceNum::new(u32::MAX),
        MonoTimestamp::from_nanos(5),
    );
    assert!(applied(&submit(&mut engine, client, at_max, 10)));
    let wrapped = frame(
        session,
        generation,
        SequenceNum::new(0),
        MonoTimestamp::from_nanos(6),
    );
    let outcome = submit(&mut engine, client, wrapped, 11);
    assert!(
        applied(&outcome),
        "the wrapped sequence advances: {:?}",
        outcome.actions
    );
}

/// A fresh generation restarts the sequence domain: the re-leased holder
/// starts small without tripping the previous generation's watermark.
#[test]
fn a_new_generation_opens_a_fresh_sequence_domain() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    let generation = grant(&mut engine, client, 1);
    let high = frame(
        session,
        generation,
        SequenceNum::new(500),
        MonoTimestamp::from_nanos(5),
    );
    assert!(applied(&submit(&mut engine, client, high, 10)));

    let released = engine.handle_client_message(
        client,
        DomainEnvelope::Release(LeaseRelease {
            vehicle: VEHICLE,
            scope: motion(),
        }),
        MonoTimestamp::from_nanos(20),
    );
    assert!(!released.actions.is_empty(), "the release is acknowledged");
    let regrant = engine.handle_client_message(
        client,
        DomainEnvelope::Lease(LeaseRequest {
            vehicle: VEHICLE,
            scope: motion(),
        }),
        MonoTimestamp::from_nanos(30),
    );
    let regeneration = regrant
        .actions
        .iter()
        .find_map(|action| match action {
            SessionAction::SendToClient {
                envelope: crate::OutboundMessage::LeaseResponse(response),
                ..
            } if response.granted => Some(response.generation),
            _ => None,
        })
        .expect("re-lease granted");
    assert!(regeneration.as_u64() > generation.as_u64());

    // Neutral (the release latched the vehicle; the fenced holder's neutral
    // demonstration is what recovery admits first) and sequence 1 — far
    // BEHIND 500, admissible only because the domain restarted.
    let restart = neutral_frame(
        session,
        regeneration,
        SequenceNum::new(1),
        MonoTimestamp::from_nanos(35),
    );
    let outcome = submit(&mut engine, client, restart, 40);
    assert!(
        applied(&outcome),
        "the new generation's low sequence is fresh: {:?}",
        outcome.actions
    );
}

/// A frame landing AFTER the holder-silence deadline but before the next
/// scheduled tick is judged against the EXPIRED lease: the deadline fires
/// first, the vehicle neutralizes, and the frame is fenced — it must never
/// refresh the deadline it already missed and resurrect dead authority.
#[test]
fn a_late_frame_after_the_deadline_cannot_resurrect_authority() {
    let mut engine = engine_with_silence(Duration::from_nanos(100));
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    let generation = grant(&mut engine, client, 1); // deadline 101

    // No tick ran; the frame itself arrives at t=150, past the deadline.
    let late = frame(
        session,
        generation,
        SequenceNum::new(1),
        MonoTimestamp::from_nanos(150),
    );
    let outcome = submit(&mut engine, client, late, 150);
    assert!(
        link_lost(&outcome),
        "the missed deadline is enforced in the SAME call: {:?}",
        outcome.actions
    );
    assert_eq!(engaged_neutralize(&outcome), 1);
    assert!(
        !applied(&outcome),
        "the late frame must not drive the vehicle: {:?}",
        outcome.actions
    );
    assert_eq!(
        rejection_reason(&outcome),
        Some(FrameRejectionReason::NoHolder),
        "the lease was already released when the frame was judged: {:?}",
        outcome.actions
    );

    // The loss already happened; a later tick has nothing left to engage.
    let tick = engine.handle_tick(MonoTimestamp::from_nanos(200));
    assert_eq!(engaged_neutralize(&tick), 0);
}

/// The client's sample clock shares no epoch with the host clock: a sender
/// whose clock reads a minute behind stays fresh as long as its frames keep
/// arriving within the staleness window of its OWN observed floor.
#[test]
fn a_skewed_client_clock_is_correlated_not_rejected() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    let generation = grant(&mut engine, client, 59_999_999_000);

    // Host clock is ~60 s ahead of the client's sample clock.
    let first = frame(
        session,
        generation,
        SequenceNum::new(1),
        MonoTimestamp::from_nanos(5_000),
    );
    let outcome = submit(&mut engine, client, first, 60_000_000_000);
    assert!(
        applied(&outcome),
        "the floor-defining frame is fresh: {:?}",
        outcome.actions
    );

    // 10 ms later on both clocks (plus 5 µs of path delay): still fresh,
    // because the age is judged against the correlated floor — a raw
    // subtraction would read a minute of staleness.
    let second = frame(
        session,
        generation,
        SequenceNum::new(2),
        MonoTimestamp::from_nanos(10_000_000),
    );
    let outcome = submit(&mut engine, client, second, 60_010_005_000);
    assert!(
        applied(&outcome),
        "consistent skew is not staleness: {:?}",
        outcome.actions
    );
}
