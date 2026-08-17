//! Motion scaling, the activation announcement, video framing, and the
//! takeover choreography. The admission and fixture plumbing lives in
//! the parent module.

#![allow(clippy::expect_used, clippy::panic)]

use pilotage_protocol::wire;
use prost::Message;

use super::{WELCOME_FIXTURE, admit, engine, fixture_hex, grant};
use crate::{ClientAction, ClientEngine, ControlCommand, ModuleEvent, StreamId, TransportEvent};

/// Feeds one authority event through the session-events stream.
fn authority_event(
    engine: &mut ClientEngine,
    event: wire::authority_event::Event,
) -> Vec<ClientAction> {
    let envelope = wire::Envelope {
        schema_version: 1,
        payload: Some(wire::envelope::Payload::AuthorityEvent(
            wire::AuthorityEvent { event: Some(event) },
        )),
    };
    let bytes = pilotage_protocol::encode_envelope_length_delimited(&envelope);
    engine.handle(TransportEvent::UniStreamOpened(StreamId(9)), 0);
    let mut tagged = vec![0x01];
    tagged.extend_from_slice(&bytes);
    engine.handle(TransportEvent::UniStreamReceived(StreamId(9), tagged), 0)
}

#[test]
fn a_full_climb_demand_scales_onto_the_fixture_envelope() {
    // The shared typed-frame fixture was produced by the browser from a
    // full climb demand against the 3.0/1.5/0.9 advertised envelope; the
    // native scaling must reach the same setpoint.
    let envelope_bytes = fixture_hex(WELCOME_FIXTURE, "envelopeHex");
    let mut engine = engine();
    engine.handle(TransportEvent::Connected, 0);
    engine.handle(TransportEvent::BootstrapReceived(envelope_bytes), 0);
    let admission = engine.admission().expect("fixture admits").clone();

    let capability = crate::intent_capability(
        &admission,
        1,
        "vehicle.motion",
        wire::IntentFamily::Velocity,
    );
    let intent = crate::velocity_intent(
        crate::MotionDemand {
            roll: 0.0,
            pitch: 0.0,
            throttle: 1.0,
            yaw: 0.0,
        },
        capability,
    )
    .expect("the advertised scope builds an intent");
    let Some(wire::control_intent::Family::Velocity(velocity)) = intent.family else {
        panic!("a velocity capability builds a velocity intent");
    };
    assert!(
        (velocity.vz - (-1.5)).abs() < f32::EPSILON,
        "full climb is -maxVertical"
    );
    assert!((velocity.vx).abs() < f32::EPSILON);

    // No advertisement, no intent: fail closed.
    assert!(
        crate::velocity_intent(
            crate::MotionDemand {
                roll: 0.0,
                pitch: 0.0,
                throttle: 1.0,
                yaw: 0.0
            },
            None
        )
        .is_none()
    );
}

#[test]
fn the_first_grant_announces_the_profile_and_binds_the_lane() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    engine.request_lease(1, "vehicle.motion");
    let actions = grant(&mut engine, 4);

    // The grant's actions carry exactly one activation announcement.
    let activation = actions
        .iter()
        .find_map(|action| match action {
            ClientAction::SendBootstrap(bytes) => {
                let (envelope, _) =
                    pilotage_protocol::decode_envelope_length_delimited(bytes).ok()?;
                match envelope.payload {
                    Some(wire::envelope::Payload::ProfileActivation(activation)) => {
                        Some(activation)
                    }
                    _ => None,
                }
            }
            _ => None,
        })
        .expect("the first grant announces the control profile");
    assert_eq!(activation.session.map(|s| s.value), Some(7));
    assert_eq!(activation.activation_revision, 1);
    assert_eq!(activation.digest.len(), 32);

    // Every later frame binds to the announced activation, or the host
    // rejects the press with "activation revision does not match".
    let actions = engine.control_frame(ControlCommand::Legacy(wire::ControlPayload::default()), 10);
    let ClientAction::SendDatagram(bytes) = &actions[0] else {
        panic!("a held lease produces a datagram");
    };
    let envelope = wire::Envelope::decode(bytes.as_slice()).expect("frame decodes");
    let Some(wire::envelope::Payload::ControlFrame(frame)) = envelope.payload else {
        panic!("the datagram is a control frame");
    };
    assert_eq!(frame.activation_revision, 1);
}

#[test]
fn a_video_stream_yields_frame_bodies_across_split_reads() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    engine.handle(TransportEvent::UniStreamOpened(StreamId(3)), 0);

    let body = vec![0xAB_u8; 10];
    let mut wire_bytes = vec![0x04];
    wire_bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
    wire_bytes.extend_from_slice(&body);

    let first = engine.handle(
        TransportEvent::UniStreamReceived(StreamId(3), wire_bytes[..6].to_vec()),
        0,
    );
    assert!(first.is_empty(), "a partial record emits nothing");
    let rest = engine.handle(
        TransportEvent::UniStreamReceived(StreamId(3), wire_bytes[6..].to_vec()),
        0,
    );
    let ClientAction::Emit(ModuleEvent::VideoFrame(received)) = &rest[0] else {
        panic!("a completed record emits one video frame body");
    };
    assert_eq!(received, &body);
}

#[test]
fn a_takeover_accepts_the_answering_offer_and_opens_the_lane_on_commit() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);

    let actions = engine.request_takeover(1, "vehicle.motion");
    assert!(matches!(actions[0], ClientAction::SendBootstrap(_)));
    assert!(!engine.holds_control());

    // The holder's offer addressed to this principal is accepted without
    // a hand on the screen: the ask was the operator's decision.
    let actions = authority_event(
        &mut engine,
        wire::authority_event::Event::ScopeTransferOffered(wire::ScopeTransferOffered {
            from_principal: Some(wire::PrincipalId { value: 9 }),
            to_principal: Some(wire::PrincipalId { value: 42 }),
            vehicle: Some(wire::VehicleId { value: 1 }),
            scope: Some(wire::ScopeId {
                value: "vehicle.motion".into(),
            }),
            generation: Some(wire::Generation { value: 5 }),
            reason: String::new(),
            authority_class: 0,
        }),
    );
    let accepted = actions.iter().any(|action| match action {
        ClientAction::SendBootstrap(bytes) => {
            pilotage_protocol::decode_envelope_length_delimited(bytes)
                .ok()
                .is_some_and(|(envelope, _)| {
                    matches!(
                        envelope.payload,
                        Some(wire::envelope::Payload::ScopeTransferAccept(_))
                    )
                })
        }
        _ => false,
    });
    assert!(accepted, "the answering offer is accepted");

    // An offer addressed to someone else is not.
    let actions = authority_event(
        &mut engine,
        wire::authority_event::Event::ScopeTransferOffered(wire::ScopeTransferOffered {
            from_principal: Some(wire::PrincipalId { value: 9 }),
            to_principal: Some(wire::PrincipalId { value: 8 }),
            vehicle: Some(wire::VehicleId { value: 1 }),
            scope: Some(wire::ScopeId {
                value: "vehicle.motion".into(),
            }),
            generation: Some(wire::Generation { value: 5 }),
            reason: String::new(),
            authority_class: 0,
        }),
    );
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, ClientAction::SendBootstrap(_))),
        "someone else's offer is not touched"
    );

    // The commit choreography continues in the sibling test.
}

#[test]
fn a_committed_transfer_opens_the_lane_and_a_departing_one_closes_it() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    engine.request_takeover(1, "vehicle.motion");

    // The commit opens the lane at the committed generation.
    authority_event(
        &mut engine,
        wire::authority_event::Event::ScopeTransferCommitted(wire::ScopeTransferCommitted {
            from_principal: Some(wire::PrincipalId { value: 9 }),
            to_principal: Some(wire::PrincipalId { value: 42 }),
            vehicle: Some(wire::VehicleId { value: 1 }),
            scope: Some(wire::ScopeId {
                value: "vehicle.motion".into(),
            }),
            generation: Some(wire::Generation { value: 6 }),
            reason: String::new(),
            authority_class: 0,
        }),
    );
    assert!(
        engine.holds_control(),
        "the committed transfer arms control"
    );
    let frame_actions =
        engine.control_frame(ControlCommand::Legacy(wire::ControlPayload::default()), 10);
    let ClientAction::SendDatagram(bytes) = &frame_actions[0] else {
        panic!("the transferred lane sends frames");
    };
    let envelope = wire::Envelope::decode(bytes.as_slice()).expect("frame decodes");
    let Some(wire::envelope::Payload::ControlFrame(frame)) = envelope.payload else {
        panic!("a control frame");
    };
    assert_eq!(frame.generation.as_ref().map(|g| g.value), Some(6));

    // Authority moving away closes the lane and says so.
    let actions = authority_event(
        &mut engine,
        wire::authority_event::Event::ScopeTransferCommitted(wire::ScopeTransferCommitted {
            from_principal: Some(wire::PrincipalId { value: 42 }),
            to_principal: Some(wire::PrincipalId { value: 9 }),
            vehicle: Some(wire::VehicleId { value: 1 }),
            scope: Some(wire::ScopeId {
                value: "vehicle.motion".into(),
            }),
            generation: Some(wire::Generation { value: 7 }),
            reason: String::new(),
            authority_class: 0,
        }),
    );
    assert!(!engine.holds_control(), "moved authority closes the lane");
    assert!(actions.iter().any(|action| matches!(
        action,
        ClientAction::Emit(ModuleEvent::Lease(response)) if !response.granted
    )));
}

#[test]
fn a_freed_scope_turns_a_pending_ask_into_a_plain_lease_request() {
    // The holder can stand down instead of offering — a blur latch, a
    // release button, a disconnect. The asker must not wait forever for
    // an offer no one is left to make.
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    engine.request_takeover(1, "vehicle.motion");

    let actions = authority_event(
        &mut engine,
        wire::authority_event::Event::ScopeLeaseRevoked(wire::ScopeLeaseRevoked {
            principal: Some(wire::PrincipalId { value: 9 }),
            vehicle: Some(wire::VehicleId { value: 1 }),
            scope: Some(wire::ScopeId {
                value: "vehicle.motion".into(),
            }),
            generation: Some(wire::Generation { value: 2 }),
            reason: String::new(),
            authority_class: 0,
        }),
    );
    let requested = actions.iter().any(|action| match action {
        ClientAction::SendBootstrap(bytes) => {
            pilotage_protocol::decode_envelope_length_delimited(bytes)
                .ok()
                .is_some_and(|(envelope, _)| {
                    matches!(
                        envelope.payload,
                        Some(wire::envelope::Payload::LeaseRequest(_))
                    )
                })
        }
        _ => false,
    });
    assert!(requested, "the pending ask becomes a lease request");

    // The grant then arms control exactly like a first-hand lease.
    grant(&mut engine, 3);
    assert!(engine.holds_control());
}

#[test]
fn a_denied_request_becomes_the_ask_without_a_second_press() {
    // One operator intent, one flow: request control; if someone holds
    // it, the ask goes out by itself, and the holder's offer finishes
    // the job.
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    engine.request_lease(1, "vehicle.motion");

    let denial = wire::Envelope {
        schema_version: 1,
        payload: Some(wire::envelope::Payload::LeaseResponse(
            wire::LeaseResponse {
                vehicle: Some(wire::VehicleId { value: 1 }),
                scope: Some(wire::ScopeId {
                    value: "vehicle.motion".into(),
                }),
                granted: false,
                generation: Some(wire::Generation { value: 2 }),
                reason: 1,
            },
        )),
    };
    let bytes = pilotage_protocol::encode_envelope_length_delimited(&denial);
    let actions = engine.handle(TransportEvent::BootstrapReceived(bytes), 0);
    let asked = actions.iter().any(|action| match action {
        ClientAction::SendBootstrap(sent) => {
            pilotage_protocol::decode_envelope_length_delimited(sent)
                .ok()
                .is_some_and(|(envelope, _)| {
                    matches!(
                        envelope.payload,
                        Some(wire::envelope::Payload::ScopeTransferRequest(_))
                    )
                })
        }
        _ => None::<()>.is_some(),
    });
    assert!(asked, "the holder-present denial escalates to the ask");

    // The committed transfer then arms control, end of the same flow.
    authority_event(
        &mut engine,
        wire::authority_event::Event::ScopeTransferCommitted(wire::ScopeTransferCommitted {
            from_principal: Some(wire::PrincipalId { value: 9 }),
            to_principal: Some(wire::PrincipalId { value: 42 }),
            vehicle: Some(wire::VehicleId { value: 1 }),
            scope: Some(wire::ScopeId {
                value: "vehicle.motion".into(),
            }),
            generation: Some(wire::Generation { value: 3 }),
            reason: String::new(),
            authority_class: 0,
        }),
    );
    assert!(engine.holds_control());
}

#[test]
fn a_revoked_lane_closes_instead_of_commanding_a_stale_generation() {
    // The host's silence watchdog revoked the lease one second after a
    // grant that never produced a frame; the operator's screen kept
    // saying "controlling" and every later press carried the dead
    // generation.
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    engine.request_lease(1, "vehicle.motion");
    grant(&mut engine, 4);
    assert!(engine.holds_control());

    let actions = authority_event(
        &mut engine,
        wire::authority_event::Event::ScopeLeaseRevoked(wire::ScopeLeaseRevoked {
            principal: Some(wire::PrincipalId { value: 42 }),
            vehicle: Some(wire::VehicleId { value: 1 }),
            scope: Some(wire::ScopeId {
                value: "vehicle.motion".into(),
            }),
            generation: Some(wire::Generation { value: 5 }),
            reason: "holder silence".into(),
            authority_class: 0,
        }),
    );
    assert!(!engine.holds_control(), "the revoked lane is gone");
    assert!(actions.iter().any(|action| matches!(
        action,
        ClientAction::Emit(ModuleEvent::Lease(response)) if !response.granted
    )));
    assert!(
        engine
            .control_frame(ControlCommand::Legacy(wire::ControlPayload::default()), 1)
            .is_empty(),
        "no frame leaves under a dead generation"
    );
}
