//! Wire clients for admission regression tests.

#![cfg(test)]

use pilotage_protocol::{JoinToken, wire};
use pilotage_session_host::cli::AdapterKind;
use pilotage_session_host::runtime::{NetworkOptions, RunningHost, RuntimeOptions};
use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;
use wtransport::{Connection, Endpoint, RecvStream, SendStream};

pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

pub async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(8), future)
        .await
        .unwrap()
}

pub fn token() -> JoinToken {
    JoinToken::from_hex(&"ab".repeat(32)).unwrap()
}

pub async fn start_host(join_token: Option<JoinToken>) -> RunningHost {
    pilotage_session_host::runtime::start_with_options(
        0,
        AdapterKind::Reference,
        RuntimeOptions {
            network: NetworkOptions {
                join_token,
                ..NetworkOptions::default()
            },
            ..RuntimeOptions::default()
        },
    )
    .await
    .unwrap()
}

pub async fn connect(address: SocketAddr) -> Connection {
    let endpoint = Endpoint::client(
        wtransport::ClientConfig::builder()
            .with_bind_default()
            .with_no_cert_validation()
            .build(),
    )
    .unwrap();
    bounded(endpoint.connect(format!("https://127.0.0.1:{}/pilotage", address.port())))
        .await
        .unwrap()
}

pub struct Client {
    pub connection: Connection,
    pub send: SendStream,
    recv: RecvStream,
    pending: Vec<u8>,
}

impl Client {
    pub async fn connect(address: SocketAddr) -> Self {
        let connection = connect(address).await;
        let (send, recv) = bounded(connection.open_bi()).await.unwrap().await.unwrap();
        Self {
            connection,
            send,
            recv,
            pending: Vec::new(),
        }
    }

    pub async fn welcome(&mut self) -> wire::ServerWelcome {
        let frame = read_frame(&mut self.recv, &mut self.pending).await;
        let Some(wire::envelope::Payload::ServerWelcome(welcome)) = frame.payload else {
            panic!("first server frame must be Welcome");
        };
        welcome
    }

    pub async fn lease_response(&mut self) -> wire::LeaseResponse {
        loop {
            let frame = read_frame(&mut self.recv, &mut self.pending).await;
            if let Some(wire::envelope::Payload::LeaseResponse(lease)) = frame.payload {
                return lease;
            }
        }
    }

    pub async fn assert_rejected_without_state(&mut self) {
        let mut bytes = [0; 4096];
        let (closed, read, uni, datagram) = bounded(async {
            tokio::join!(
                self.connection.closed(),
                self.recv.read(&mut bytes),
                self.connection.accept_uni(),
                self.connection.receive_datagram(),
            )
        })
        .await;
        super::assert_rejection(closed);
        assert!(!matches!(read, Ok(Some(count)) if count > 0));
        assert!(
            uni.is_err(),
            "no authority or media stream before admission"
        );
        assert!(datagram.is_err(), "no telemetry before admission");
    }
}

pub fn hello(token: Option<&JoinToken>) -> Vec<u8> {
    encode(wire::envelope::Payload::ClientHello(wire::ClientHello {
        protocol_version: pilotage_protocol::SESSION_PROTOCOL_VERSION,
        client_name: "admission-test".into(),
        join_token: token.map_or_else(Vec::new, |value| value.as_bytes().to_vec()),
    }))
}

pub fn lease() -> Vec<u8> {
    encode(wire::envelope::Payload::LeaseRequest(wire::LeaseRequest {
        vehicle: Some(wire::VehicleId { value: 1 }),
        scope: Some(wire::ScopeId {
            value: "vehicle.motion".into(),
        }),
    }))
}

pub fn media_attach() -> Vec<u8> {
    encode(wire::envelope::Payload::MediaAttachRequest(
        wire::MediaAttachRequest {},
    ))
}

fn encode(payload: wire::envelope::Payload) -> Vec<u8> {
    pilotage_protocol::encode_envelope_length_delimited(&wire::Envelope {
        schema_version: pilotage_protocol::SCHEMA_VERSION,
        payload: Some(payload),
    })
}

pub async fn read_frame(recv: &mut RecvStream, pending: &mut Vec<u8>) -> wire::Envelope {
    bounded(async {
        loop {
            if let Ok((frame, rest)) = pilotage_protocol::decode_envelope_length_delimited(pending)
            {
                let used = pending.len() - rest.len();
                pending.drain(..used);
                return frame;
            }
            let mut bytes = [0; 8192];
            let count = recv.read(&mut bytes).await.unwrap().unwrap();
            pending.extend_from_slice(&bytes[..count]);
        }
    })
    .await
}
