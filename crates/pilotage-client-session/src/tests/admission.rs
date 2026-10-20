use super::*;
use crate::ClientFault;
use pilotage_protocol::JoinToken;

#[test]
fn every_hello_carries_the_configured_token() {
    let token = JoinToken::from_hex(&"a5".repeat(32)).unwrap();
    let mut engine = ClientEngine::new(ClientConfig {
        client_name: "admission".into(),
        reconnect: ReconnectPolicy::default(),
        join_token: Some(token.clone()),
    });
    for time in [0, 1_000] {
        let actions = engine.handle(TransportEvent::Connected, time);
        let [ClientAction::SendBootstrap(bytes)] = actions.as_slice() else {
            panic!("hello is the first action");
        };
        let (envelope, remainder) =
            pilotage_protocol::decode_envelope_length_delimited(bytes).unwrap();
        assert!(remainder.is_empty());
        let Some(wire::envelope::Payload::ClientHello(hello)) = envelope.payload else {
            panic!("hello payload");
        };
        assert!(token.matches(&hello.join_token));
        engine.handle(
            TransportEvent::TransportLost {
                detail: "test".into(),
            },
            time,
        );
    }
}

#[test]
fn rejected_admission_clears_authority_and_cannot_reconnect_or_process_late_data() {
    let mut engine = engine();
    admit(&mut engine, 4, 9);
    engine.request_lease(1, "vehicle.motion");
    grant(&mut engine, 7);
    engine.request_lease(1, "vehicle.camera");
    assert!(engine.holds_control());
    let actions = engine.handle(TransportEvent::AdmissionRejected, 500);
    assert_eq!(
        actions,
        vec![
            ClientAction::Emit(ModuleEvent::ConnectionDown { retry_at_ms: None }),
            ClientAction::Stop(ClientFault::AdmissionRejected),
        ]
    );
    assert_eq!(*engine.phase(), ClientPhase::Stopped);
    assert!(engine.admission().is_none());
    assert!(!engine.holds_control());
    assert!(engine.pending_leases.is_empty());
    assert!(engine.pending_takeover.is_none());
    assert_eq!(engine.authority(), &crate::AuthorityMirror::default());
    for event in [
        TransportEvent::Connected,
        TransportEvent::TransportLost {
            detail: "closed".into(),
        },
        TransportEvent::BootstrapReceived(vec![0]),
        TransportEvent::UniStreamOpened(StreamId(3)),
        TransportEvent::DatagramReceived(vec![0]),
    ] {
        assert!(engine.handle(event, 1_000).is_empty());
    }
    assert_eq!(engine.stream_pending_bytes(), 0);
}
