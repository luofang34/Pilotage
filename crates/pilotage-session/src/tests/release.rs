//! Voluntary lease release (ADR-0006): a holder relinquishes a scope
//! explicitly — the generation advances (stragglers are fenced), the
//! vehicle's link-loss policy engages exactly as for an involuntary loss,
//! and the sender gets a typed acknowledgement, so an immediate re-grant
//! never races the silence watchdog into `AlreadyHeld`.

use core::time::Duration;

use pilotage_protocol::{LeaseRelease, SequenceNum};
use pilotage_timing::MonoTimestamp;

use super::{
    VEHICLE, cleared, engaged_neutralize, engine_with_silence, frame, grant, motion, neutral_frame,
    welcome,
};
use crate::{ClientKey, DomainEnvelope, OutboundMessage, SessionAction, SessionOutcome};

fn release_ack(outcome: &SessionOutcome) -> Option<(bool, u64)> {
    outcome.actions.iter().find_map(|action| match action {
        SessionAction::SendToClient {
            envelope: OutboundMessage::LeaseReleased(ack),
            ..
        } => Some((ack.released, ack.generation.as_u64())),
        _ => None,
    })
}

fn release_envelope() -> DomainEnvelope {
    DomainEnvelope::Release(LeaseRelease {
        vehicle: VEHICLE,
        scope: motion(),
    })
}

#[test]
fn a_holder_release_is_acknowledged_fenced_and_neutralized() {
    let mut engine = engine_with_silence(Duration::from_nanos(100));
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    let generation = grant(&mut engine, client, 1);

    let outcome =
        engine.handle_client_message(client, release_envelope(), MonoTimestamp::from_nanos(10));
    let (released, ack_generation) = release_ack(&outcome).expect("acknowledgement sent");
    assert!(released, "the holder's release succeeds");
    assert!(
        ack_generation > generation.as_u64(),
        "the acknowledged generation is the advanced fence"
    );
    assert_eq!(
        engaged_neutralize(&outcome),
        1,
        "a voluntary release still neutralizes: {:?}",
        outcome.actions
    );

    // A straggler frame at the pre-release generation is fenced.
    let late = engine.handle_client_message(
        client,
        DomainEnvelope::Frame(frame(
            session,
            generation,
            SequenceNum::new(5),
            MonoTimestamp::from_nanos(20),
        )),
        MonoTimestamp::from_nanos(20),
    );
    assert!(
        !late
            .actions
            .iter()
            .any(|a| matches!(a, SessionAction::ApplyToAdapter { .. })),
        "a straggler after release must be fenced: {:?}",
        late.actions
    );
}

#[test]
fn a_non_holder_release_is_acknowledged_as_a_no_op() {
    let mut engine = engine_with_silence(Duration::from_nanos(100));
    let holder = ClientKey::new(1);
    let bystander = ClientKey::new(2);
    welcome(&mut engine, holder);
    welcome(&mut engine, bystander);
    grant(&mut engine, holder, 1);

    let outcome =
        engine.handle_client_message(bystander, release_envelope(), MonoTimestamp::from_nanos(10));
    let (released, _) = release_ack(&outcome).expect("acknowledgement sent");
    assert!(!released, "a bystander releases nothing");
    assert_eq!(
        engaged_neutralize(&outcome),
        0,
        "nothing was lost, nothing engages: {:?}",
        outcome.actions
    );
}

#[test]
fn release_then_immediate_regrant_never_races_already_held() {
    // The AlreadyHeld race the explicit release exists to close: with only
    // the silence watchdog, a client reconnecting faster than the window
    // finds its old self still holding the scope. After an acknowledged
    // release the scope is free IMMEDIATELY.
    let mut engine = engine_with_silence(Duration::from_secs(1));
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    grant(&mut engine, client, 1);

    let released =
        engine.handle_client_message(client, release_envelope(), MonoTimestamp::from_nanos(10));
    assert_eq!(release_ack(&released).map(|(ok, _)| ok), Some(true));

    // Re-grant in the very next message — no watchdog wait, no AlreadyHeld.
    let regrant_generation = grant(&mut engine, client, 20);

    // The engaged policy still clears only through the activation frame.
    let neutral = engine.handle_client_message(
        client,
        DomainEnvelope::Frame(neutral_frame(
            session,
            regrant_generation,
            SequenceNum::new(1),
            MonoTimestamp::from_nanos(30),
        )),
        MonoTimestamp::from_nanos(30),
    );
    assert_eq!(
        cleared(&neutral),
        1,
        "recovery completes under the fresh grant: {:?}",
        neutral.actions
    );
}
