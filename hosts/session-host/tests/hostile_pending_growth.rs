//! Guardrail: a client that declares an oversized bootstrap-stream frame is
//! disconnected without growing the host's per-connection reassembly buffer
//! toward the attacker-chosen size, and the host keeps serving other clients
//! (ADR-0005 bootstrap stream; `wire_codec::MAX_BOOTSTRAP_FRAME_LEN`).
//!
//! Synchronization is event-driven: the pass condition is that a fresh
//! connection completes hello->welcome after the hostile one, bounded by a
//! `tokio::time::timeout` — never a sleep-and-poll.

#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use pilotage_protocol::wire;
use pilotage_session_host::cli::AdapterKind;
use pilotage_session_host::runtime;
use tokio::time::timeout;
use wtransport::{ClientConfig, Connection, Endpoint};

const SCHEMA_VERSION: u32 = 1;
const TEST_TIMEOUT: Duration = Duration::from_secs(5);

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

fn encode_varint(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
}

fn hello_bytes() -> Vec<u8> {
    let envelope = wire::Envelope {
        schema_version: SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::ClientHello(wire::ClientHello {
            protocol_version: SCHEMA_VERSION,
            client_name: "post-attack-client".to_owned(),
            join_token: Vec::new(),
        })),
    };
    pilotage_protocol::encode_envelope_length_delimited(&envelope)
}

async fn read_welcome(recv: &mut wtransport::RecvStream) -> wire::ServerWelcome {
    let mut pending = Vec::new();
    let mut buf = vec![0u8; 8192];
    loop {
        if let Ok((envelope, rest)) = pilotage_protocol::decode_envelope_length_delimited(&pending)
        {
            let consumed = pending.len() - rest.len();
            pending.drain(..consumed);
            if let Some(wire::envelope::Payload::ServerWelcome(welcome)) = envelope.payload {
                return welcome;
            }
            continue;
        }
        let read = timeout(TEST_TIMEOUT, recv.read(&mut buf))
            .await
            .expect("welcome read does not time out")
            .expect("welcome read succeeds")
            .expect("stream stays open until a full welcome arrives");
        pending.extend_from_slice(&buf[..read]);
    }
}

/// A hostile client declares a 1 GiB bootstrap frame and streams a little
/// body; the host must close that connection instead of buffering toward the
/// declared size, and must still welcome a subsequent honest client.
#[tokio::test]
async fn oversized_bootstrap_frame_is_rejected_and_host_survives() {
    let host = runtime::start(0, AdapterKind::Reference)
        .await
        .expect("host starts on an ephemeral port");
    let addr = host.local_addr;

    // Hostile connection: declare a frame far past the cap, then dribble body.
    let hostile = connect_client(addr).await;
    let (mut hostile_send, mut hostile_recv) = timeout(TEST_TIMEOUT, hostile.open_bi())
        .await
        .expect("open_bi does not time out")
        .expect("bootstrap stream opens")
        .await
        .expect("bootstrap stream finishes opening");
    let mut prefix = Vec::new();
    encode_varint(1 << 30, &mut prefix);
    hostile_send
        .write_all(&prefix)
        .await
        .expect("prefix write is accepted by the transport");
    // A single body chunk, well under the declared size; the host should have
    // already rejected on the prefix alone, so this write may fail once the
    // stream is reset — that outcome is expected, not asserted.
    hostile_send.write_all(&[0u8; 4096]).await.ok();

    // The host closes the hostile bootstrap stream: its receive side ends
    // (reset or clean close) rather than blocking forever.
    let mut scratch = vec![0u8; 1024];
    let hostile_stream_closed = timeout(TEST_TIMEOUT, hostile_recv.read(&mut scratch))
        .await
        .expect("hostile stream resolves (closed) within the timeout");
    assert!(
        matches!(hostile_stream_closed, Ok(None) | Err(_)),
        "host must close the hostile bootstrap stream, got {hostile_stream_closed:?}"
    );

    // The decisive guarantee: an honest client is still served, proving the
    // host neither exhausted memory nor wedged its accept path.
    let honest = connect_client(addr).await;
    let (mut honest_send, mut honest_recv) = timeout(TEST_TIMEOUT, honest.open_bi())
        .await
        .expect("second open_bi does not time out")
        .expect("second bootstrap stream opens")
        .await
        .expect("second bootstrap stream finishes opening");
    let _authority = timeout(TEST_TIMEOUT, honest.accept_uni())
        .await
        .expect("accept_uni does not time out")
        .expect("authority stream is accepted for the honest client");

    honest_send
        .write_all(&hello_bytes())
        .await
        .expect("honest hello write succeeds");
    let welcome = read_welcome(&mut honest_recv).await;
    assert!(
        welcome.session.is_some(),
        "honest client receives a session after the hostile connection was rejected"
    );

    host.shutdown().await;
}
