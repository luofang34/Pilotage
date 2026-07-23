//! Real WebTransport coverage for allocated stream cleanup and credit reuse.

#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use pilotage_session::ClientKey;
use tokio::sync::{Semaphore, mpsc};
use tokio::time::timeout;
use wtransport::config::QuicTransportConfig;
use wtransport::quinn::VarInt as QuinnVarInt;
use wtransport::stream::OpeningUniStream;
use wtransport::{ClientConfig, Connection, Endpoint, Identity, SendStream, ServerConfig, VarInt};

use super::reaper::OpenReapers;
use super::{
    DeadlinePhase, FrameChannel, FrameOutcome, FrameStream, StreamError, classify_open,
    classify_open_request, classify_write, deliver_frame,
};
use crate::runtime::media::budget::PressureSignals;

const IO_BOUND: Duration = Duration::from_secs(5);
const SOAK_CYCLES: usize = 32;

struct GatedConnection {
    connection: Connection,
    header_gates: Arc<Semaphore>,
    reset_events: mpsc::UnboundedSender<bool>,
}

struct GatedOpening {
    opening: OpeningUniStream,
    header_gates: Arc<Semaphore>,
    reset_events: mpsc::UnboundedSender<bool>,
}

struct ObservedSendStream {
    stream: SendStream,
    reset_events: mpsc::UnboundedSender<bool>,
}

impl FrameStream for ObservedSendStream {
    fn set_priority(&self, priority: i32) {
        self.stream.set_priority(priority);
    }

    async fn write_all(&mut self, buf: &[u8]) -> Result<(), StreamError> {
        self.stream
            .write_all(buf)
            .await
            .map_err(|error| classify_write(&error, "write"))
    }

    async fn finish(&mut self) -> Result<(), StreamError> {
        self.stream
            .finish()
            .await
            .map_err(|error| classify_write(&error, "finish"))
    }

    fn reset(&mut self) {
        let reset = self
            .stream
            .reset(VarInt::from_u32(super::STALL_RESET_CODE))
            .is_ok();
        self.reset_events.send(reset).ok();
    }
}

impl FrameChannel for GatedConnection {
    type Stream = ObservedSendStream;
    type Opening = GatedOpening;

    async fn request_open(&self) -> Result<GatedOpening, StreamError> {
        let opening = self
            .connection
            .open_uni()
            .await
            .map_err(classify_open_request)?;
        Ok(GatedOpening {
            opening,
            header_gates: Arc::clone(&self.header_gates),
            reset_events: self.reset_events.clone(),
        })
    }

    async fn finish_open(opening: GatedOpening) -> Result<ObservedSendStream, StreamError> {
        let permit = opening
            .header_gates
            .acquire_owned()
            .await
            .expect("test gate remains open");
        permit.forget();
        let stream = opening
            .opening
            .await
            .map_err(|error| classify_open(&error))?;
        Ok(ObservedSendStream {
            stream,
            reset_events: opening.reset_events,
        })
    }
}

async fn connected_pair() -> (Connection, Connection) {
    let identity =
        Identity::self_signed(["localhost", "127.0.0.1"]).expect("test identity constructs");
    let server_config = ServerConfig::builder()
        .with_bind_address(([127, 0, 0, 1], 0).into())
        .with_identity(identity)
        .build();
    let server = Endpoint::server(server_config).expect("server endpoint binds");
    let server_addr = server.local_addr().expect("server has a local address");

    let mut client_config = ClientConfig::builder()
        .with_bind_default()
        .with_no_cert_validation()
        .build();
    let mut transport = QuicTransportConfig::default();
    transport.max_concurrent_uni_streams(QuinnVarInt::from_u32(8));
    client_config
        .quic_config_mut()
        .transport_config(Arc::new(transport));
    let client = Endpoint::client(client_config).expect("client endpoint binds");
    let url = format!("https://127.0.0.1:{}/media-test", server_addr.port());

    let accept = async {
        server
            .accept()
            .await
            .await
            .expect("session request arrives")
            .accept()
            .await
            .expect("server accepts session")
    };
    let connect = async {
        client
            .connect(url)
            .await
            .expect("client connects to server")
    };
    timeout(IO_BOUND, async { tokio::join!(accept, connect) })
        .await
        .expect("connection setup stays bounded")
}

#[tokio::test]
async fn repeated_mid_open_deadlines_recycle_real_quic_credit() {
    let (server, client) = connected_pair().await;
    let header_gates = Arc::new(Semaphore::new(0));
    let (reset_tx, mut reset_rx) = mpsc::unbounded_channel();
    let channel = GatedConnection {
        connection: server,
        header_gates: Arc::clone(&header_gates),
        reset_events: reset_tx,
    };
    let mut reapers = OpenReapers::new(ClientKey::new(21), 4, Arc::new(PressureSignals::default()));

    for _ in 0..SOAK_CYCLES {
        let outcome = deliver_frame(
            &channel,
            &mut reapers,
            IO_BOUND,
            Duration::ZERO,
            0x02,
            b"abandoned-frame",
        )
        .await;
        assert!(
            matches!(
                outcome,
                FrameOutcome::Stalled {
                    phase: DeadlinePhase::HeaderFlush,
                    reset: false,
                    reaper_owned: true,
                }
            ),
            "the allocated open transfers to its reaper: {outcome:?}"
        );
        header_gates.add_permits(1);
        let reset = timeout(IO_BOUND, reset_rx.recv())
            .await
            .expect("the allocated stream is reaped")
            .expect("the reset observer remains connected");
        assert!(reset, "the real QUIC stream accepts RESET_STREAM");
    }

    header_gates.add_permits(1);
    let outcome = deliver_frame(
        &channel,
        &mut reapers,
        IO_BOUND,
        IO_BOUND,
        0x02,
        b"healthy-frame",
    )
    .await;
    assert!(matches!(outcome, FrameOutcome::Sent));

    let mut healthy = timeout(IO_BOUND, client.accept_uni())
        .await
        .expect("healthy stream is surfaced")
        .expect("connection remains healthy");
    let mut body = [0_u8; 14];
    timeout(IO_BOUND, healthy.read_exact(&mut body))
        .await
        .expect("healthy frame arrives")
        .expect("healthy frame is complete");
    assert_eq!(&body, b"\x02healthy-frame");
    assert_eq!(
        timeout(IO_BOUND, healthy.read(&mut [0_u8; 1]))
            .await
            .expect("healthy FIN arrives")
            .expect("healthy stream has no read error"),
        None,
        "the healthy frame finishes cleanly"
    );
}
