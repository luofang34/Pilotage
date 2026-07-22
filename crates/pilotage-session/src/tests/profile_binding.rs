//! Profile traceability enforcement (INPUT-01): typed frames must bind to
//! the sender's announced profile activation, announcements must name the
//! sender's own session, and the activation revision must advance
//! monotonically — otherwise control evidence cannot trace a frame back to
//! the exact mapping that produced it.

use pilotage_protocol::{
    ControlIntent, ControlPayload, FrameRejectionReason, Generation, ReferenceFrame,
    ScopedControlFrame, SequenceNum, SessionId, VelocityIntent,
};
use pilotage_timing::MonoTimestamp;

use super::{VEHICLE, engine, frame, motion, welcome};
use crate::{ClientKey, CloseReason, DomainEnvelope, SessionAction, SessionEngine};

fn activation(session: SessionId, activation_revision: u32) -> DomainEnvelope {
    DomainEnvelope::ProfileActivation(pilotage_protocol::ProfileActivation {
        session,
        profile_id: "builtin.flight.default".to_owned(),
        profile_revision: 1,
        activation_revision,
        digest: [0x11; 32],
        device_profile_id: String::new(),
        device_profile_revision: 0,
        device_digest: [0; 32],
    })
}

/// A typed velocity frame (no legacy payload) carrying `activation_revision`.
fn typed_frame(session: SessionId, activation_revision: u32) -> ScopedControlFrame {
    ScopedControlFrame {
        session,
        vehicle: VEHICLE,
        scope: motion(),
        generation: Generation::new(1),
        sequence: SequenceNum::new(0),
        sampled_at: MonoTimestamp::from_nanos(1_000),
        profile_revision: 1,
        activation_revision,
        payload: ControlPayload {
            axes: vec![],
            edges: vec![],
        },
        intent: Some(ControlIntent::Velocity(VelocityIntent {
            frame: ReferenceFrame::BodyFrd,
            vx: 0.5,
            vy: 0.0,
            vz: 0.0,
            yaw_rate: 0.0,
        })),
        actions: vec![],
        action_ids: vec![],
    }
}

fn welcomed_holder(engine: &mut SessionEngine, client: ClientKey) -> SessionId {
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
) -> Vec<SessionAction> {
    engine
        .handle_client_message(
            client,
            DomainEnvelope::Frame(frame),
            MonoTimestamp::from_nanos(1_010),
        )
        .actions
}

#[test]
fn an_unannounced_typed_frame_is_rejected_as_profile_mismatch() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    let session = welcomed_holder(&mut engine, client);
    let actions = submit(&mut engine, client, typed_frame(session, 1));
    match actions.as_slice() {
        [SessionAction::RejectFrame { rejection, .. }] => {
            assert_eq!(rejection.reason, FrameRejectionReason::ProfileMismatch);
        }
        other => panic!("expected a profile-mismatch rejection, got {other:?}"),
    }
}

#[test]
fn a_typed_frame_with_a_stale_activation_revision_is_rejected() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    let session = welcomed_holder(&mut engine, client);
    let announced =
        engine.handle_client_message(client, activation(session, 2), MonoTimestamp::from_nanos(2));
    assert!(
        matches!(
            announced.actions.as_slice(),
            [SessionAction::ActivationAccepted { .. }]
        ),
        "the engine emits its explicit acceptance event: {:?}",
        announced.actions
    );
    // A frame still stamped with the PREVIOUS activation revision cannot be
    // traced to the mapping now in effect.
    let actions = submit(&mut engine, client, typed_frame(session, 1));
    match actions.as_slice() {
        [SessionAction::RejectFrame { rejection, .. }] => {
            assert_eq!(rejection.reason, FrameRejectionReason::ProfileMismatch);
        }
        other => panic!("expected a profile-mismatch rejection, got {other:?}"),
    }
}

#[test]
fn a_typed_frame_matching_the_announced_activation_is_applied() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    let session = welcomed_holder(&mut engine, client);
    let announced =
        engine.handle_client_message(client, activation(session, 3), MonoTimestamp::from_nanos(2));
    assert!(
        matches!(
            announced.actions.as_slice(),
            [SessionAction::ActivationAccepted { .. }]
        ),
        "the engine emits its explicit acceptance event: {:?}",
        announced.actions
    );
    let actions = submit(&mut engine, client, typed_frame(session, 3));
    assert!(
        matches!(actions.as_slice(), [SessionAction::ApplyToAdapter { .. }]),
        "a bound typed frame reaches the adapter, got {actions:?}"
    );
}

#[test]
fn a_legacy_payload_frame_needs_no_announcement() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    let session = welcomed_holder(&mut engine, client);
    // The loopback probe predates profiles: legacy payload frames carry
    // activation revision 0 and stay exempt from the binding check.
    let legacy = frame(
        session,
        Generation::new(1),
        SequenceNum::new(0),
        MonoTimestamp::from_nanos(1_000),
    );
    let actions = submit(&mut engine, client, legacy);
    assert!(matches!(
        actions.as_slice(),
        [SessionAction::ApplyToAdapter { .. }]
    ));
}

#[test]
fn an_activation_naming_a_foreign_session_closes_the_client() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    let foreign = SessionId::new(session.as_u64().wrapping_add(99));
    let outcome =
        engine.handle_client_message(client, activation(foreign, 1), MonoTimestamp::from_nanos(2));
    match outcome.actions.as_slice() {
        [SessionAction::CloseClient { reason, .. }] => {
            assert!(
                matches!(reason, CloseReason::ProfileSessionMismatch { .. }),
                "got {reason:?}"
            );
        }
        other => panic!("expected a close, got {other:?}"),
    }
    assert!(
        engine.active_profile(client).is_none(),
        "a forged announcement must never be recorded"
    );
}

#[test]
fn a_non_advancing_activation_revision_closes_the_client() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    let announced =
        engine.handle_client_message(client, activation(session, 5), MonoTimestamp::from_nanos(2));
    assert!(
        matches!(
            announced.actions.as_slice(),
            [SessionAction::ActivationAccepted { .. }]
        ),
        "the engine emits its explicit acceptance event: {:?}",
        announced.actions
    );
    for stale in [5u32, 4u32] {
        let outcome = engine.handle_client_message(
            client,
            activation(session, stale),
            MonoTimestamp::from_nanos(3),
        );
        assert!(
            outcome.actions.iter().any(|action| matches!(
                action,
                SessionAction::CloseClient {
                    reason: CloseReason::NonMonotonicActivation { .. },
                    ..
                }
            )),
            "revision {stale} after 5 must close, got {:?}",
            outcome.actions
        );
    }
}

#[test]
fn the_activation_revision_advances_across_the_u32_wrap() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    let announced = engine.handle_client_message(
        client,
        activation(session, u32::MAX),
        MonoTimestamp::from_nanos(2),
    );
    assert!(
        matches!(
            announced.actions.as_slice(),
            [SessionAction::ActivationAccepted { .. }]
        ),
        "the engine emits its explicit acceptance event: {:?}",
        announced.actions
    );
    // u32::MAX → 0 is a forward step of 1 under wrapping arithmetic; a
    // long-lived sender must not be closed for surviving the wrap.
    let outcome =
        engine.handle_client_message(client, activation(session, 0), MonoTimestamp::from_nanos(3));
    assert!(
        matches!(
            outcome.actions.as_slice(),
            [SessionAction::ActivationAccepted { .. }]
        ),
        "the wrap advance is recorded, got {:?}",
        outcome.actions
    );
    let recorded = engine.active_profile(client).expect("recorded");
    assert_eq!(recorded.activation_revision, 0);
}

#[test]
fn acceptance_is_an_explicit_event_and_a_rejected_duplicate_emits_none() {
    // Evidence derives from the engine's explicit acceptance event. A
    // duplicate (non-advancing) announcement is REJECTED — it closes the
    // client and must not emit an acceptance, even though the standing
    // record still carries the same revision (the state comparison an
    // actor must never fall back to).
    let mut engine = engine();
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    let first =
        engine.handle_client_message(client, activation(session, 1), MonoTimestamp::from_nanos(2));
    assert!(matches!(
        first.actions.as_slice(),
        [SessionAction::ActivationAccepted { .. }]
    ));
    let duplicate =
        engine.handle_client_message(client, activation(session, 1), MonoTimestamp::from_nanos(3));
    assert!(
        !duplicate
            .actions
            .iter()
            .any(|action| matches!(action, SessionAction::ActivationAccepted { .. })),
        "a rejected duplicate must never look accepted: {:?}",
        duplicate.actions
    );
    assert!(
        duplicate
            .actions
            .iter()
            .any(|action| matches!(action, SessionAction::CloseClient { .. }))
    );
}
