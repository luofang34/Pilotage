//! Ping/pong: the responder echoes the nonce and sender sample and stamps its
//! own reply time (ADR-0009).

use pilotage_protocol::Ping;
use pilotage_timing::MonoTimestamp;

use super::{engine, welcome};
use crate::{ClientKey, DomainEnvelope, OutboundMessage, SessionAction};

#[test]
fn ping_yields_pong_echoing_nonce_and_stamping_responder_time() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    welcome(&mut engine, client);
    let ping = Ping {
        nonce: 0xABCD,
        sender_sent_at: MonoTimestamp::from_nanos(1_234),
    };
    let now = MonoTimestamp::from_nanos(9_999);
    let outcome = engine.handle_client_message(client, DomainEnvelope::Ping(ping), now);
    match outcome.actions.as_slice() {
        [
            SessionAction::SendToClient {
                envelope: OutboundMessage::Pong(pong),
                ..
            },
        ] => {
            assert_eq!(pong.nonce, 0xABCD);
            assert_eq!(pong.echoed_sender_sent_at.as_nanos(), 1_234);
            assert_eq!(pong.responder_sent_at.as_nanos(), 9_999);
        }
        other => panic!("expected a pong, got {other:?}"),
    }
}

#[test]
fn ping_before_handshake_closes_the_connection() {
    let mut engine = engine();
    let ping = Ping {
        nonce: 1,
        sender_sent_at: MonoTimestamp::from_nanos(0),
    };
    let outcome = engine.handle_client_message(
        ClientKey::new(1),
        DomainEnvelope::Ping(ping),
        MonoTimestamp::from_nanos(0),
    );
    assert!(matches!(
        outcome.actions.as_slice(),
        [SessionAction::CloseClient { .. }]
    ));
}
