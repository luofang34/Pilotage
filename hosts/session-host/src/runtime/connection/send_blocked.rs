//! Send-path stall forensics for a live connection.
//!
//! The wedge family this names: every stream write — even a 9-byte header
//! flush — stops completing while datagrams keep flowing. quinn never
//! transmits `DATA_BLOCKED` frames (its tx counter stays zero by
//! construction), so the observable signature is the pair of counters that
//! go silent together when the peer stops consuming: `frame_tx.STREAM`
//! (our stream frames out) and `frame_rx.MAX_DATA` (the peer's
//! flow-control crediting in). A connection that was actively streaming
//! and then shows an interval with NEITHER advancing is starved by its
//! peer — not congestion, which instead shows losses and a collapsed
//! congestion window in the same snapshot.

use std::time::Duration;

use tracing::{info, warn};
use wtransport::Connection;

/// Sampling cadence for the flow counters.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);

/// STREAM frames per interval below which the connection counts as idle
/// chatter (bootstrap, session events) rather than active media flow; only
/// an ACTIVE flow that hard-stops is a stall.
const ACTIVE_STREAM_FLOOR: u64 = 30;

/// One sampled view of the counters that matter for send-path stalls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct FlowSample {
    /// STREAM frames transmitted (cumulative).
    pub(super) stream_tx: u64,
    /// MAX_DATA frames received — the peer's connection-window crediting.
    pub(super) max_data_rx: u64,
}

impl FlowSample {
    pub(super) fn take(connection: &Connection) -> Self {
        let stats = connection.quic_connection().stats();
        Self {
            stream_tx: stats.frame_tx.stream,
            max_data_rx: stats.frame_rx.max_data,
        }
    }
}

/// Where the connection's send path stands in the stall state machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FlowState {
    /// No active stream flow observed yet (or only idle chatter).
    Idle,
    /// Stream frames were flowing at media rate in a recent interval.
    Active,
    /// An active flow hard-stopped with the peer's crediting silent too.
    Stalled,
}

/// Advances the state machine by one sampled interval. A stall requires a
/// previously ACTIVE flow plus an interval where nothing went out and no
/// credit came in; small nonzero intervals keep the current state.
pub(super) fn next_flow_state(
    state: FlowState,
    prev: FlowSample,
    current: FlowSample,
) -> FlowState {
    let stream_delta = current.stream_tx.saturating_sub(prev.stream_tx);
    let credit_delta = current.max_data_rx.saturating_sub(prev.max_data_rx);
    if stream_delta >= ACTIVE_STREAM_FLOOR {
        return FlowState::Active;
    }
    if stream_delta == 0 && credit_delta == 0 && state != FlowState::Idle {
        return FlowState::Stalled;
    }
    state
}

/// Watches the connection's flow counters until dropped, logging the
/// stall/recovery transitions with congestion context so an incident log
/// names the starvation shape. Runs alongside the reader/writer in the
/// connection's `select!` and ends with them.
pub(super) async fn run_send_blocked_watch(connection: &Connection, client: u64) {
    let mut state = FlowState::Idle;
    let mut prev = FlowSample::take(connection);
    loop {
        tokio::time::sleep(SAMPLE_INTERVAL).await;
        let current = FlowSample::take(connection);
        let next = next_flow_state(state, prev, current);
        if state != FlowState::Stalled && next == FlowState::Stalled {
            let stats = connection.quic_connection().stats();
            warn!(
                client,
                stream_tx = current.stream_tx,
                max_data_rx = current.max_data_rx,
                cwnd = stats.path.cwnd,
                lost_packets = stats.path.lost_packets,
                congestion_events = stats.path.congestion_events,
                "send path stalled: no stream frames out and no MAX_DATA credit in \
                 for a full interval (peer stopped consuming)"
            );
        } else if state == FlowState::Stalled && next == FlowState::Active {
            info!(
                client,
                "send path recovered: stream flow and peer crediting resumed"
            );
        }
        state = next;
        prev = current;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use std::sync::Arc;
    use std::time::Duration;

    use tokio::time::timeout;
    use wtransport::config::QuicTransportConfig;
    use wtransport::quinn::VarInt as QuinnVarInt;
    use wtransport::{ClientConfig, Connection, Endpoint, Identity, ServerConfig};

    use super::{FlowSample, FlowState, next_flow_state};

    const IO_BOUND: Duration = Duration::from_secs(8);

    // ---- state machine ----------------------------------------------------

    #[test]
    fn idle_chatter_never_reaches_stalled() {
        let mut state = FlowState::Idle;
        // Bootstrap sends a handful of stream frames, then silence.
        state = next_flow_state(state, sample(0, 0), sample(8, 1));
        state = next_flow_state(state, sample(8, 1), sample(8, 1));
        assert_eq!(state, FlowState::Idle, "an idle session is not a stall");
    }

    #[test]
    fn an_active_flow_that_hard_stops_stalls_and_recovers() {
        let mut state = FlowState::Idle;
        state = next_flow_state(state, sample(0, 0), sample(500, 6));
        assert_eq!(state, FlowState::Active);
        // The wedge-onset interval still flushes a few frames…
        state = next_flow_state(state, sample(500, 6), sample(510, 6));
        assert_eq!(
            state,
            FlowState::Active,
            "a trickle keeps the current state"
        );
        // …then a full interval with nothing out and no credit in.
        state = next_flow_state(state, sample(510, 6), sample(510, 6));
        assert_eq!(state, FlowState::Stalled);
        // Frames flowing again at media rate is recovery.
        state = next_flow_state(state, sample(510, 6), sample(900, 9));
        assert_eq!(state, FlowState::Active);
    }

    fn sample(stream_tx: u64, max_data_rx: u64) -> FlowSample {
        FlowSample {
            stream_tx,
            max_data_rx,
        }
    }

    // ---- the real counters under real starvation --------------------------

    /// A loopback pair whose CLIENT grants only a tiny connection-level
    /// receive window — the geometry of the freeze incident, where the
    /// peer's application stops consuming and `MAX_DATA` stops advancing.
    async fn starved_pair() -> (Connection, Connection) {
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
        transport.receive_window(QuinnVarInt::from_u32(64 * 1024));
        client_config
            .quic_config_mut()
            .transport_config(Arc::new(transport));
        let client = Endpoint::client(client_config).expect("client endpoint binds");
        let url = format!("https://127.0.0.1:{}/send-blocked-test", server_addr.port());
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
        let connect = async { client.connect(url).await.expect("client connects") };
        timeout(IO_BOUND, async { tokio::join!(accept, connect) })
            .await
            .expect("connection setup stays bounded")
    }

    #[tokio::test]
    async fn a_peer_that_stops_consuming_silences_stream_tx_and_max_data_rx() {
        let (server, client) = starved_pair().await;
        let start = FlowSample::take(&server);

        // Fill the peer's whole connection window while it reads NOTHING;
        // the write blocks once the window is exhausted.
        let mut stream = timeout(IO_BOUND, server.open_uni())
            .await
            .expect("open credit within bound")
            .expect("open accepted")
            .await
            .expect("stream opens");
        let payload = vec![0_u8; 512 * 1024];
        let write = tokio::time::timeout(Duration::from_secs(2), stream.write_all(&payload)).await;
        assert!(write.is_err(), "the write must block on the starved window");

        let blocked = FlowSample::take(&server);
        let active_delta = blocked.stream_tx - start.stream_tx;
        assert!(
            active_delta >= super::ACTIVE_STREAM_FLOOR,
            "filling the window is an active interval ({active_delta} frames)"
        );
        assert_eq!(
            blocked.max_data_rx, start.max_data_rx,
            "no credit ever arrived"
        );

        // A further interval while blocked moves NOTHING: the stall shape.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let still = FlowSample::take(&server);
        assert_eq!(still, blocked, "blocked flow is fully silent");
        assert_eq!(
            next_flow_state(FlowState::Active, blocked, still),
            FlowState::Stalled,
            "the real starved counters drive the state machine to Stalled"
        );
        drop(client);
    }
}
