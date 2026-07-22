//! The reliable action chain over the REAL transport (CTRL-01): a live
//! WebTransport host, a real bootstrap stream, and a real adapter. The
//! interleavings the datagram channel could never make safe are pinned
//! here end to end: a fully bound command reaches the adapter and answers
//! with a correlated result on the ordered stream; a replayed command
//! re-answers without re-executing; and a command bound to a superseded
//! generation — the delayed-Arm-after-Disarm shape — is refused.

#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use pilotage_protocol::wire;
use pilotage_session_host::cli::AdapterKind;
use pilotage_session_host::runtime;
use prost::Message;
use tokio::time::timeout;
use wtransport::{ClientConfig, Connection, Endpoint};

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

async fn connect_client(addr: std::net::SocketAddr) -> Connection {
    let config = ClientConfig::builder()
        .with_bind_default()
        .with_no_cert_validation()
        .build();
    let url = format!("https://127.0.0.1:{}", addr.port());
    timeout(
        TEST_TIMEOUT,
        Endpoint::client(config).expect("client").connect(url),
    )
    .await
    .expect("connect does not time out")
    .expect("connection establishes")
}

async fn send_envelope(send: &mut wtransport::SendStream, envelope: &wire::Envelope) {
    let bytes = envelope.encode_length_delimited_to_vec();
    send.write_all(&bytes)
        .await
        .expect("bootstrap stream write succeeds");
}

async fn read_one_envelope(
    recv: &mut wtransport::RecvStream,
    pending: &mut Vec<u8>,
) -> wire::Envelope {
    loop {
        if let Ok(envelope) = wire::Envelope::decode_length_delimited(pending.as_slice()) {
            let consumed =
                envelope.encoded_len() + prost::length_delimiter_len(envelope.encoded_len());
            pending.drain(..consumed);
            return envelope;
        }
        let mut chunk = [0u8; 1024];
        let read = timeout(TEST_TIMEOUT, recv.read(&mut chunk))
            .await
            .expect("bootstrap read does not time out")
            .expect("bootstrap read succeeds")
            .expect("bootstrap stream stays open");
        pending.extend_from_slice(&chunk[..read]);
    }
}

/// Reads envelopes until a `ControlActionResult` arrives, returning it.
async fn read_until_action_result(
    recv: &mut wtransport::RecvStream,
    pending: &mut Vec<u8>,
) -> wire::ControlActionResult {
    loop {
        let envelope = read_one_envelope(recv, pending).await;
        if let Some(wire::envelope::Payload::ControlActionResult(result)) = envelope.payload {
            return result;
        }
    }
}

async fn read_until_lease_response(
    recv: &mut wtransport::RecvStream,
    pending: &mut Vec<u8>,
) -> wire::LeaseResponse {
    loop {
        let envelope = read_one_envelope(recv, pending).await;
        if let Some(wire::envelope::Payload::LeaseResponse(response)) = envelope.payload {
            return response;
        }
    }
}

fn envelope(payload: wire::envelope::Payload) -> wire::Envelope {
    wire::Envelope {
        schema_version: 1,
        payload: Some(payload),
    }
}

fn action_command(
    session: u64,
    vehicle: u64,
    scope: &str,
    generation: u64,
    action_id: u32,
) -> wire::Envelope {
    envelope(wire::envelope::Payload::ControlActionCommand(
        wire::ControlActionCommand {
            session: Some(wire::SessionId { value: session }),
            vehicle: Some(wire::VehicleId { value: vehicle }),
            scope: Some(wire::ScopeId {
                value: scope.to_owned(),
            }),
            generation: Some(wire::Generation { value: generation }),
            activation_revision: 1,
            request: Some(wire::ControlActionRequest {
                action: wire::ControlAction::Arm as i32,
                mode_target: 0,
                action_id,
            }),
        },
    ))
}

/// A live session bound for actions: handshake done, activation revision 1
/// announced BEFORE the lease on the same ordered stream (INPUT-01), and
/// the motion lease granted.
struct BoundSession {
    send: wtransport::SendStream,
    recv: wtransport::RecvStream,
    pending: Vec<u8>,
    session: u64,
    vehicle: u64,
    generation: u64,
}

async fn bind_session(connection: &Connection) -> BoundSession {
    let (mut send, mut recv) = timeout(TEST_TIMEOUT, connection.open_bi())
        .await
        .expect("open_bi does not time out")
        .expect("bootstrap stream opens")
        .await
        .expect("bootstrap stream finishes opening");
    let mut pending = Vec::new();
    send_envelope(
        &mut send,
        &envelope(wire::envelope::Payload::ClientHello(wire::ClientHello {
            protocol_version: 1,
            client_name: "reliable-actions".to_owned(),
            join_token: vec![],
        })),
    )
    .await;
    let welcome = match read_one_envelope(&mut recv, &mut pending).await.payload {
        Some(wire::envelope::Payload::ServerWelcome(welcome)) => welcome,
        other => panic!("expected ServerWelcome, got {other:?}"),
    };
    let session = welcome.session.expect("session id").value;
    let vehicle = welcome
        .host_capabilities
        .expect("capabilities")
        .vehicles
        .first()
        .expect("one vehicle")
        .vehicle
        .expect("vehicle id")
        .value;
    send_envelope(
        &mut send,
        &envelope(wire::envelope::Payload::ProfileActivation(
            wire::ProfileActivation {
                session: Some(wire::SessionId { value: session }),
                profile_id: "builtin.flight.default".to_owned(),
                profile_revision: 1,
                activation_revision: 1,
                digest: vec![0x11; 32],
                device_profile_id: String::new(),
                device_profile_revision: 0,
                device_digest: vec![],
            },
        )),
    )
    .await;
    send_envelope(
        &mut send,
        &envelope(wire::envelope::Payload::LeaseRequest(wire::LeaseRequest {
            vehicle: Some(wire::VehicleId { value: vehicle }),
            scope: Some(wire::ScopeId {
                value: pilotage_adapter_reference::MOTION_SCOPE.to_owned(),
            }),
        })),
    )
    .await;
    let lease = read_until_lease_response(&mut recv, &mut pending).await;
    assert!(lease.granted);
    let generation = lease.generation.expect("generation").value;
    BoundSession {
        send,
        recv,
        pending,
        session,
        vehicle,
        generation,
    }
}

/// Releases and reacquires the scope, returning the fresh generation.
async fn refence(
    send: &mut wtransport::SendStream,
    recv: &mut wtransport::RecvStream,
    pending: &mut Vec<u8>,
    vehicle: u64,
    scope: &str,
) -> u64 {
    send_envelope(
        send,
        &envelope(wire::envelope::Payload::LeaseRelease(wire::LeaseRelease {
            vehicle: Some(wire::VehicleId { value: vehicle }),
            scope: Some(wire::ScopeId {
                value: scope.to_owned(),
            }),
        })),
    )
    .await;
    send_envelope(
        send,
        &envelope(wire::envelope::Payload::LeaseRequest(wire::LeaseRequest {
            vehicle: Some(wire::VehicleId { value: vehicle }),
            scope: Some(wire::ScopeId {
                value: scope.to_owned(),
            }),
        })),
    )
    .await;
    // Skip the LeaseReleased ack, then take the fresh grant.
    let regrant = read_until_lease_response(recv, pending).await;
    assert!(regrant.granted);
    regrant.generation.expect("generation").value
}

#[tokio::test]
async fn the_reliable_action_chain_holds_over_the_real_transport() {
    let host = runtime::start(0, AdapterKind::Reference)
        .await
        .expect("host starts on an ephemeral port");
    let connection = connect_client(host.local_addr).await;
    let BoundSession {
        mut send,
        mut recv,
        mut pending,
        session,
        vehicle,
        generation,
    } = bind_session(&connection).await;
    let scope = pilotage_adapter_reference::MOTION_SCOPE;

    // 1. A fully bound command reaches the adapter and answers with a
    //    CORRELATED result on the reliable stream.
    send_envelope(
        &mut send,
        &action_command(session, vehicle, scope, generation, 1),
    )
    .await;
    let result = read_until_action_result(&mut recv, &mut pending).await;
    assert_eq!(result.action_id, 1, "the result echoes the correlation id");
    assert!(
        result.accepted,
        "the reference vehicle acknowledges ARM: {result:?}"
    );

    // 2. A replay of the SAME command re-answers without re-executing (the
    //    host's dedup owns exactly-once; the answer is the proof the sender
    //    needs).
    send_envelope(
        &mut send,
        &action_command(session, vehicle, scope, generation, 1),
    )
    .await;
    let replayed = read_until_action_result(&mut recv, &mut pending).await;
    assert_eq!(replayed.action_id, 1);
    assert!(replayed.accepted, "the cached result is replayed");

    // 3. The delayed-Arm shape: authority is re-fenced (release + regrant),
    //    then a command still bound to the OLD generation arrives. It must
    //    be refused with an explicit rejected result — never executed.
    let fresh = refence(&mut send, &mut recv, &mut pending, vehicle, scope).await;
    assert!(fresh > generation, "the fence advanced");

    send_envelope(
        &mut send,
        &action_command(session, vehicle, scope, generation, 2),
    )
    .await;
    let stale = read_until_action_result(&mut recv, &mut pending).await;
    assert_eq!(stale.action_id, 2);
    assert!(!stale.accepted, "the stale-bound arm is refused: {stale:?}");
    assert!(
        stale.detail.contains("stale generation"),
        "the refusal names its reason: {}",
        stale.detail
    );

    host.shutdown().await;
}
