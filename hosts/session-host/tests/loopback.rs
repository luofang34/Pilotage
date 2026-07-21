//! In-process loopback integration test (ADR-0005): starts the session host
//! on `127.0.0.1:0`, connects a real `wtransport` client, drives
//! hello->welcome, a lease, a valid control frame observed as a telemetry
//! change, and a stale-generation frame rejected with `FrameRejected`.
//!
//! Synchronization is entirely event-driven: every wait is either a protocol
//! response the server must send, or a bounded `tokio::time::timeout` around
//! that wait (never a bare `sleep` + retry poll).

#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use pilotage_protocol::wire;
use pilotage_session_host::cli::AdapterKind;
use pilotage_session_host::runtime;
use prost::Message;
use tokio::time::timeout;
use wtransport::{ClientConfig, Connection, Endpoint};

const SCHEMA_VERSION: u32 = 1;
const TEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Connects a `wtransport` client to `addr`, skipping certificate
/// validation: the host's self-signed loopback-dev certificate is not
/// otherwise pinned in this test (ADR-0005's local-demo strategy is a
/// tracked follow-up, not this test's concern).
async fn connect_client(addr: std::net::SocketAddr) -> Connection {
    let config = ClientConfig::builder()
        .with_bind_default()
        .with_no_cert_validation()
        .build();
    let client = Endpoint::client(config).expect("client endpoint constructs");
    let url = format!("https://127.0.0.1:{}/pilotage", addr.port());
    timeout(TEST_TIMEOUT, client.connect(url))
        .await
        .expect("connect does not time out")
        .expect("client connects to the loopback host")
}

/// Reads exactly one length-delimited [`wire::Envelope`] from the bootstrap
/// stream, accumulating bytes across reads as needed. `pending` persists
/// across calls so a caller can keep reading the same stream and consume
/// each envelope exactly once, in emission order.
async fn read_one_envelope(
    recv: &mut wtransport::RecvStream,
    pending: &mut Vec<u8>,
) -> wire::Envelope {
    let mut buf = vec![0u8; 8192];
    loop {
        if let Ok((envelope, rest)) = pilotage_protocol::decode_envelope_length_delimited(pending) {
            let consumed = pending.len() - rest.len();
            pending.drain(..consumed);
            return envelope;
        }
        let read = timeout(TEST_TIMEOUT, recv.read(&mut buf))
            .await
            .expect("stream read does not time out")
            .expect("stream read succeeds")
            .expect("stream is not closed before a full envelope arrives");
        pending.extend_from_slice(&buf[..read]);
    }
}

/// Reads bootstrap-stream envelopes until one is a `LeaseResponse`. Authority
/// broadcasts (e.g. the grant this lease request causes) no longer share this
/// stream (ADR-0005's dedicated authority-events stream), so every envelope
/// read here is expected to be bootstrap/lease traffic.
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

/// Kind tag prefixing the host's authority-events uni stream (ADR-0005:
/// every host-initiated uni stream leads with a 1-byte kind tag; `0x01`
/// distinguishes authority events from `0x02` video frames).
const AUTHORITY_EVENTS_TAG: u8 = 0x01;

/// Reads the single leading kind-tag byte from a freshly accepted
/// host-initiated uni stream, asserting it identifies the authority-events
/// stream. Any bytes read past the tag are left in `pending` for the envelope
/// parser.
async fn read_authority_tag(recv: &mut wtransport::RecvStream, pending: &mut Vec<u8>) {
    let mut buf = vec![0u8; 8192];
    while pending.is_empty() {
        let read = timeout(TEST_TIMEOUT, recv.read(&mut buf))
            .await
            .expect("stream read does not time out")
            .expect("stream read succeeds")
            .expect("authority stream is not closed before its kind tag arrives");
        pending.extend_from_slice(&buf[..read]);
    }
    let tag = pending.remove(0);
    assert_eq!(
        tag, AUTHORITY_EVENTS_TAG,
        "authority-events stream must lead with its kind tag"
    );
}

/// Reads authority-events-stream envelopes until one is a
/// `ScopeLeaseGranted` event, skipping any other authority events that
/// precede it.
async fn read_until_lease_granted(
    recv: &mut wtransport::RecvStream,
    pending: &mut Vec<u8>,
) -> wire::AuthorityEvent {
    loop {
        let envelope = read_one_envelope(recv, pending).await;
        if let Some(wire::envelope::Payload::AuthorityEvent(event)) = envelope.payload
            && matches!(
                event.event,
                Some(wire::authority_event::Event::ScopeLeaseGranted(_))
            )
        {
            return event;
        }
    }
}

/// Sends one envelope, length-delimited, on the bootstrap stream.
async fn send_envelope(send: &mut wtransport::SendStream, envelope: &wire::Envelope) {
    let bytes = pilotage_protocol::encode_envelope_length_delimited(envelope);
    send.write_all(&bytes)
        .await
        .expect("bootstrap write succeeds");
}

fn hello_envelope() -> wire::Envelope {
    wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::ClientHello(wire::ClientHello {
            protocol_version: SCHEMA_VERSION,
            client_name: "loopback-test".to_owned(),
            join_token: Vec::new(),
        })),
    }
}

fn lease_envelope(vehicle: u64, scope: &str) -> wire::Envelope {
    wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::LeaseRequest(wire::LeaseRequest {
            vehicle: Some(wire::VehicleId { value: vehicle }),
            scope: Some(wire::ScopeId {
                value: scope.to_owned(),
            }),
        })),
    }
}

/// Builds a `ControlFrame` envelope for the datagram channel with full
/// throttle, so the reference adapter's speed observably departs from zero.
fn full_throttle_frame_bytes(
    session: u64,
    vehicle: u64,
    scope: &str,
    generation: u64,
    sequence: u32,
    sampled_at_nanos: u64,
) -> Vec<u8> {
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::ControlFrame(wire::ControlFrame {
            session: Some(wire::SessionId { value: session }),
            vehicle: Some(wire::VehicleId { value: vehicle }),
            scope: Some(wire::ScopeId {
                value: scope.to_owned(),
            }),
            generation: Some(wire::Generation { value: generation }),
            sequence: Some(wire::SequenceNum { value: sequence }),
            sampled_at: Some(wire::MonoTimestamp {
                nanos: sampled_at_nanos,
            }),
            profile_revision: 1,
            payload: Some(wire::ControlPayload {
                axes: vec![wire::AxisSample {
                    axis_id: u32::from(pilotage_adapter_reference::THROTTLE_AXIS),
                    value: 1.0,
                }],
                edges: Vec::new(),
            }),
            intent: None,
            actions: Vec::new(),
        })),
    };
    envelope.encode_to_vec()
}

/// Awaits telemetry datagrams until one reports nonzero speed (proof the
/// control frame was applied), or the test timeout elapses.
async fn await_nonzero_speed_telemetry(connection: &Connection) -> wire::TelemetrySample {
    timeout(TEST_TIMEOUT, async {
        loop {
            let datagram = connection
                .receive_datagram()
                .await
                .expect("datagram channel stays open");
            let Ok(envelope) = wire::Envelope::decode(datagram.payload().as_ref()) else {
                continue;
            };
            if let Some(wire::envelope::Payload::TelemetrySample(sample)) = envelope.payload {
                let speed = sample
                    .velocity
                    .as_ref()
                    .map_or(0.0, |velocity| velocity.linear_x_mps);
                if speed > 0.0 {
                    return sample;
                }
            }
        }
    })
    .await
    .expect("a nonzero-speed telemetry sample arrives before the test timeout")
}

/// Awaits a `FrameRejected` datagram, or the test timeout elapses.
async fn await_frame_rejected(connection: &Connection) -> wire::FrameRejected {
    timeout(TEST_TIMEOUT, async {
        loop {
            let datagram = connection
                .receive_datagram()
                .await
                .expect("datagram channel stays open");
            let Ok(envelope) = wire::Envelope::decode(datagram.payload().as_ref()) else {
                continue;
            };
            if let Some(wire::envelope::Payload::FrameRejected(rejection)) = envelope.payload {
                return rejection;
            }
        }
    })
    .await
    .expect("a FrameRejected datagram arrives before the test timeout")
}

#[tokio::test]
async fn hello_lease_frame_and_stale_generation_rejection() {
    let host = runtime::start(0, AdapterKind::Reference)
        .await
        .expect("host starts on an ephemeral port");
    let addr = host.local_addr;

    let connection = connect_client(addr).await;
    let (mut send, mut recv) = timeout(TEST_TIMEOUT, connection.open_bi())
        .await
        .expect("open_bi does not time out")
        .expect("bootstrap stream opens")
        .await
        .expect("bootstrap stream finishes opening");
    let mut authority_recv = timeout(TEST_TIMEOUT, connection.accept_uni())
        .await
        .expect("accept_uni does not time out")
        .expect("dedicated authority-events stream is accepted");

    let mut pending = Vec::new();
    let mut authority_pending = Vec::new();

    // The authority-events uni stream now leads with a 1-byte kind tag
    // (ADR-0005); consume it before parsing length-delimited envelopes.
    read_authority_tag(&mut authority_recv, &mut authority_pending).await;

    send_envelope(&mut send, &hello_envelope()).await;
    let welcome_envelope = read_one_envelope(&mut recv, &mut pending).await;
    let welcome = match welcome_envelope.payload {
        Some(wire::envelope::Payload::ServerWelcome(welcome)) => welcome,
        other => panic!("expected ServerWelcome, got {other:?}"),
    };
    let session = welcome.session.expect("welcome carries a session id").value;
    let vehicle_id = welcome
        .host_capabilities
        .expect("welcome carries host capabilities")
        .vehicles
        .first()
        .expect("reference adapter advertises one vehicle")
        .vehicle
        .expect("vehicle descriptor carries an id")
        .value;
    let scope = pilotage_adapter_reference::MOTION_SCOPE;

    send_envelope(&mut send, &lease_envelope(vehicle_id, scope)).await;
    let lease_response = read_until_lease_response(&mut recv, &mut pending).await;
    assert!(lease_response.granted, "lease request should be granted");
    let generation = lease_response
        .generation
        .expect("lease response carries a generation")
        .value;

    // The grant's authority broadcast arrives on the dedicated
    // authority-events stream (ADR-0005), never sharing the bootstrap
    // stream's head-of-line with the unicast `LeaseResponse` above.
    let granted_event = timeout(
        TEST_TIMEOUT,
        read_until_lease_granted(&mut authority_recv, &mut authority_pending),
    )
    .await
    .expect("a ScopeLeaseGranted authority event arrives before the test timeout");
    match granted_event.event {
        Some(wire::authority_event::Event::ScopeLeaseGranted(grant)) => {
            assert_eq!(
                grant.vehicle.expect("grant carries a vehicle id").value,
                vehicle_id
            );
        }
        other => panic!("expected ScopeLeaseGranted, got {other:?}"),
    }

    let valid_frame = full_throttle_frame_bytes(session, vehicle_id, scope, generation, 0, 0);
    connection
        .send_datagram(valid_frame)
        .expect("sending the valid control frame datagram succeeds");
    let sample = await_nonzero_speed_telemetry(&connection).await;
    assert_eq!(
        sample.vehicle.expect("sample carries a vehicle id").value,
        vehicle_id
    );

    let stale_frame = full_throttle_frame_bytes(
        session,
        vehicle_id,
        scope,
        generation.saturating_sub(1),
        1,
        0,
    );
    connection
        .send_datagram(stale_frame)
        .expect("sending the stale-generation frame datagram succeeds");
    let rejection = await_frame_rejected(&connection).await;
    assert_eq!(
        rejection.reason,
        wire::FrameRejectionReason::StaleGeneration as i32
    );

    host.shutdown().await;
}
