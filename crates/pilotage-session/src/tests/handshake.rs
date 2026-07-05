//! Handshake: hello answered with welcome; version and duplicate rejections.

use pilotage_protocol::ClientHello;
use pilotage_timing::MonoTimestamp;

use super::{VEHICLE, engine, motion, welcome};
use crate::{ClientKey, CloseReason, DomainEnvelope, OutboundMessage, SessionAction};

fn hello(version: u32) -> DomainEnvelope {
    DomainEnvelope::Hello(ClientHello {
        protocol_version: version,
        client_name: "unit".to_owned(),
        join_token: Vec::new(),
    })
}

#[test]
fn hello_yields_welcome_with_capabilities_and_holders() {
    let mut engine = engine();
    let outcome =
        engine.handle_client_message(ClientKey::new(1), hello(1), MonoTimestamp::from_nanos(0));
    match outcome.actions.as_slice() {
        [
            SessionAction::SendToClient {
                client,
                envelope: OutboundMessage::Welcome(welcome),
            },
        ] => {
            assert_eq!(*client, ClientKey::new(1));
            assert_eq!(welcome.host_capabilities.host_version, "host-test");
            assert_eq!(welcome.host_capabilities.vehicles.len(), 1);
            // The lone scope starts unassigned at generation zero.
            assert_eq!(welcome.scope_holders.len(), 1);
            let snapshot = &welcome.scope_holders[0];
            assert_eq!(snapshot.vehicle, VEHICLE);
            assert_eq!(snapshot.scope, motion());
            assert_eq!(snapshot.holder, None);
            assert_eq!(snapshot.generation.as_u64(), 0);
        }
        other => panic!("expected a welcome, got {other:?}"),
    }
}

#[test]
fn each_client_gets_a_distinct_session_and_principal() {
    let mut engine = engine();
    let first = welcome(&mut engine, ClientKey::new(1));
    let second = welcome(&mut engine, ClientKey::new(2));
    assert_ne!(first.as_u64(), second.as_u64());
}

#[test]
fn stale_protocol_version_closes_the_connection() {
    let mut engine = engine();
    let outcome =
        engine.handle_client_message(ClientKey::new(1), hello(0), MonoTimestamp::from_nanos(0));
    match outcome.actions.as_slice() {
        [
            SessionAction::CloseClient {
                reason: CloseReason::UnsupportedProtocolVersion { offered, required },
                ..
            },
        ] => {
            assert_eq!(*offered, 0);
            assert_eq!(*required, 1);
        }
        other => panic!("expected an unsupported-version close, got {other:?}"),
    }
}

#[test]
fn second_hello_closes_the_connection() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    welcome(&mut engine, client);
    let outcome = engine.handle_client_message(client, hello(1), MonoTimestamp::from_nanos(0));
    assert!(matches!(
        outcome.actions.as_slice(),
        [SessionAction::CloseClient {
            reason: CloseReason::DuplicateHello,
            ..
        }]
    ));
}

#[test]
fn message_before_handshake_closes_the_connection() {
    let mut engine = engine();
    let outcome = engine.handle_client_message(
        ClientKey::new(1),
        DomainEnvelope::Lease(pilotage_protocol::LeaseRequest {
            vehicle: VEHICLE,
            scope: motion(),
        }),
        MonoTimestamp::from_nanos(0),
    );
    assert!(matches!(
        outcome.actions.as_slice(),
        [SessionAction::CloseClient {
            reason: CloseReason::HandshakeNotComplete,
            ..
        }]
    ));
}
