use pilotage_protocol::wire;
use prost::Message;

use super::{engine, grant};
use crate::{ClientAction, ClientEngine, TransportEvent};

const MOTION: &str = "vehicle.motion";
const MS: u64 = 1_000_000;

fn physical_engine() -> ClientEngine {
    let mut client = engine();
    let welcome = wire::Envelope {
        schema_version: pilotage_protocol::SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::ServerWelcome(
            wire::ServerWelcome {
                session: Some(wire::SessionId { value: 7 }),
                principal: Some(wire::PrincipalId { value: 42 }),
                host_capabilities: Some(wire::HostCapabilities {
                    physically_embodied: true,
                    control_window_ms: 250,
                    vehicles: vec![wire::VehicleDescriptor {
                        vehicle: Some(wire::VehicleId { value: 1 }),
                        scopes: vec![wire::ScopeDescriptor {
                            command_only: true,
                            scope: Some(wire::ScopeId {
                                value: MOTION.into(),
                            }),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            },
        )),
    };
    client.handle(TransportEvent::Connected, 0);
    client.handle(
        TransportEvent::BootstrapReceived(pilotage_protocol::encode_envelope_length_delimited(
            &welcome,
        )),
        0,
    );
    client
}

fn probe(client: &mut ClientEngine, now_ms: u64) -> wire::Ping {
    probe_ns(client, now_ms * MS)
}

fn probe_ns(client: &mut ClientEngine, now_ns: u64) -> wire::Ping {
    let actions = client.maintain_control_window(now_ns);
    assert_eq!(actions.len(), 1);
    let ClientAction::SendDatagram(bytes) = &actions[0] else {
        panic!("only a Ping")
    };
    let envelope = wire::Envelope::decode(bytes.as_slice()).expect("Ping envelope");
    let Some(wire::envelope::Payload::Ping(ping)) = envelope.payload else {
        panic!("no control frame")
    };
    ping
}

#[test]
fn a_fractional_millisecond_probe_accepts_a_same_millisecond_reply() {
    let mut client = physical_engine();
    client.request_lease(1, MOTION);
    grant(&mut client, 4);
    let ping = probe_ns(&mut client, 10 * MS + 500_000);
    assert_eq!(ping.sender_sent_at.as_ref().unwrap().nanos, 10 * MS);
    pong(&mut client, &ping, 1, 10);
    assert!(!client.control_action_ready(1, MOTION, 10 * MS + 499_999));
    assert!(!client.control_action_ready(1, MOTION, 10 * MS + 500_000));
    assert!(!client.control_action_ready(1, MOTION, 11 * MS - 1));
    assert!(client.control_action_ready(1, MOTION, 11 * MS));
    assert!(
        client
            .prepare_control_action_at(1, MOTION, takeoff(), 11 * MS)
            .is_some()
    );
    assert!(client.control_action_ready(1, MOTION, 260 * MS - 1));
    assert!(!client.control_action_ready(1, MOTION, 260 * MS));
}

fn pong(client: &mut ClientEngine, ping: &wire::Ping, id: u64, now_ms: u64) -> Vec<ClientAction> {
    let envelope = wire::Envelope {
        schema_version: pilotage_protocol::SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::Pong(wire::Pong {
            nonce: ping.nonce,
            echoed_sender_sent_at: ping.sender_sent_at,
            control_window_id: id,
            control_window_ms: 250,
            ..Default::default()
        })),
    };
    client.handle(
        TransportEvent::DatagramReceived(envelope.encode_to_vec()),
        now_ms,
    )
}

fn takeoff() -> wire::ControlActionRequest {
    wire::ControlActionRequest {
        action: wire::ControlAction::Takeoff as i32,
        ..Default::default()
    }
}

#[test]
fn command_only_lease_prewarms_without_motion_or_replaying_an_early_click() {
    let mut client = physical_engine();
    assert!(client.maintain_control_window(0).is_empty());
    client.request_lease(1, MOTION);
    grant(&mut client, 4);
    assert!(!client.control_action_ready(1, MOTION, MS));
    assert!(
        client
            .prepare_control_action_at(1, MOTION, takeoff(), MS)
            .is_none()
    );
    let ping = probe(&mut client, 10);
    assert!(client.maintain_control_window(50 * MS).is_empty());
    let received = pong(&mut client, &ping, 1, 30);
    assert!(!received.iter().any(|action| matches!(
        action,
        ClientAction::SendBootstrap(_) | ClientAction::SendDatagram(_)
    )));
    assert!(!client.control_action_ready(1, MOTION, 29 * MS));
    assert!(!client.control_action_ready(1, MOTION, 30 * MS));
    assert!(client.control_action_ready(1, MOTION, 31 * MS));
    assert!(
        client
            .prepare_control_action_at(1, MOTION, takeoff(), 31 * MS)
            .is_some()
    );
    assert!(!client.control_action_ready(1, MOTION, 260 * MS));
    assert!(
        client
            .prepare_control_action_at(1, MOTION, takeoff(), 260 * MS)
            .is_none()
    );
}

#[test]
fn delayed_unsolicited_and_duplicate_pongs_cannot_extend_a_window() {
    let mut client = physical_engine();
    client.request_lease(1, MOTION);
    grant(&mut client, 4);
    let first = probe(&mut client, 10);
    pong(&mut client, &first, 1, 258);
    assert!(client.control_action_ready(1, MOTION, 259 * MS));
    pong(&mut client, &first, 1, 260);
    assert!(!client.control_action_ready(1, MOTION, 260 * MS));
    let second = probe(&mut client, 300);
    pong(&mut client, &second, 2, 550);
    assert!(!client.control_action_ready(1, MOTION, 550 * MS));
    let fake = wire::Ping {
        nonce: 600 * MS,
        sender_sent_at: Some(wire::MonoTimestamp { nanos: 600 * MS }),
    };
    pong(&mut client, &fake, 3, 601);
    assert!(!client.control_action_ready(1, MOTION, 601 * MS));
}

#[test]
fn release_and_transport_loss_close_action_readiness_without_emitting_control() {
    let mut client = physical_engine();
    client.request_lease(1, MOTION);
    grant(&mut client, 4);
    let first = probe(&mut client, 10);
    pong(&mut client, &first, 1, 11);
    assert!(client.control_action_ready(1, MOTION, 12 * MS));
    client.release_lease(1, MOTION);
    assert!(!client.control_action_ready(1, MOTION, 12 * MS));
    assert!(client.maintain_control_window(120 * MS).is_empty());
    client.request_lease(1, MOTION);
    grant(&mut client, 6);
    assert!(client.control_action_ready(1, MOTION, 120 * MS));
    client.handle(
        TransportEvent::TransportLost {
            detail: "test disconnect".into(),
        },
        121,
    );
    assert!(!client.control_action_ready(1, MOTION, 122 * MS));
    assert!(client.maintain_control_window(122 * MS).is_empty());
}
