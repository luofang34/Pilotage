//! Top-level orchestration: builds the WebTransport endpoint, the embedded
//! [`SessionEngine`]/adapter, and wires accept, engine-actor, and shutdown
//! tasks together (ADR-0002, ADR-0005).
//!
//! [`SessionEngine`]: pilotage_session::SessionEngine

mod connection;
mod engine_actor;
mod registry;
mod shutdown;
mod wire_codec;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use pilotage_adapter_api::VehicleAdapter;
use pilotage_adapter_reference::ReferenceAdapter;
use pilotage_protocol::VehicleId;
use pilotage_session::{ClientKey, SessionConfig, SessionEngine};
use pilotage_timing::StalenessPolicy;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;
use tracing::{info, warn};
use wtransport::{Endpoint, Identity, ServerConfig};

use crate::error::HostError;
use crate::output::print_line;
use crate::tls_identity::build_dev_identity;
use engine_actor::{ENGINE_QUEUE_CAPACITY, EngineActor, ToEngine};

/// The vehicle the embedded reference adapter drives in this increment.
const HOST_VEHICLE: VehicleId = VehicleId::new(1);

/// Deterministic seed for the embedded reference adapter's initial state.
const ADAPTER_SEED: u64 = 0;

/// Maximum age a control frame may have before the engine rejects it as
/// stale (ADR-0009). Generous for loopback development.
const MAX_CONTROL_AGE: Duration = Duration::from_millis(250);

/// A running session host: its bound local address and a handle to request
/// shutdown and await full teardown.
pub struct RunningHost {
    /// The address the WebTransport endpoint is actually bound to (useful
    /// when constructed with an ephemeral `--port 0`).
    pub local_addr: SocketAddr,
    /// The self-signed certificate's hex SHA-256 digest, as printed on the
    /// `LISTENING` line.
    pub cert_hash_hex: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join: tokio::task::JoinHandle<()>,
}

impl RunningHost {
    /// Signals the host to shut down and waits for full teardown.
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            // A closed receiver means `run_until_shutdown` already exited
            // (e.g. it panicked); nothing left to signal.
            tx.send(()).ok();
        }
        // A join error here is a panic in `run_until_shutdown` itself,
        // already logged by tokio; shutdown has nothing further to do with
        // it.
        self.join.await.ok();
    }
}

/// Builds the embedded [`SessionEngine`] and [`ReferenceAdapter`] pair this
/// increment's host serves.
fn build_engine_and_adapter() -> (SessionEngine, ReferenceAdapter) {
    let adapter = ReferenceAdapter::from_seed(HOST_VEHICLE, ADAPTER_SEED);
    let capabilities = adapter.capabilities();
    let staleness = StalenessPolicy::new(MAX_CONTROL_AGE);
    let config = SessionConfig::new(pilotage_protocol::SCHEMA_VERSION, env!("CARGO_PKG_VERSION"));
    let engine = SessionEngine::new(capabilities, staleness, config);
    (engine, adapter)
}

/// Starts the session host bound to `127.0.0.1:port` (`0` for an OS-assigned
/// ephemeral port), prints the `LISTENING` line, and returns a handle for
/// shutdown.
///
/// # Errors
///
/// Returns [`HostError`] if the TLS identity cannot be built or the
/// WebTransport endpoint cannot bind.
pub fn start(port: u16) -> Result<RunningHost, HostError> {
    let dev_identity = build_dev_identity()?;
    let identity: Identity = dev_identity.identity;
    let cert_hash_hex = dev_identity.cert_hash_hex.clone();

    let config = ServerConfig::builder()
        .with_bind_default(port)
        .with_identity(identity)
        .keep_alive_interval(Some(Duration::from_secs(3)))
        .build();
    let endpoint = Endpoint::server(config).map_err(HostError::Endpoint)?;
    let local_addr = endpoint.local_addr().map_err(HostError::LocalAddr)?;

    print_line(&format!(
        "LISTENING {} {}",
        local_addr.port(),
        cert_hash_hex
    ));

    let (engine, adapter) = build_engine_and_adapter();
    let (engine_tx, engine_rx) = mpsc::channel::<ToEngine>(ENGINE_QUEUE_CAPACITY);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let join = tokio::spawn(run_until_shutdown(
        endpoint,
        engine,
        adapter,
        engine_tx,
        engine_rx,
        shutdown_rx,
    ));

    Ok(RunningHost {
        local_addr,
        cert_hash_hex,
        shutdown_tx: Some(shutdown_tx),
        join,
    })
}

/// Runs the accept loop and engine actor until `shutdown_rx` fires, then
/// tears down every spawned task (engine actor, accept loop, and every
/// per-connection task) within the shutdown timeout.
async fn run_until_shutdown(
    endpoint: Endpoint<wtransport::endpoint::endpoint_side::Server>,
    engine: SessionEngine,
    adapter: ReferenceAdapter,
    engine_tx: mpsc::Sender<ToEngine>,
    engine_rx: mpsc::Receiver<ToEngine>,
    shutdown_rx: oneshot::Receiver<()>,
) {
    // A single monotonic origin feeds every `host_time` comparison across
    // the host (ADR-0009): client-message receive stamps (via `accept_loop`
    // and each connection task) and the engine actor's tick/offer-expiry
    // stamps must derive from the same `Instant`, or the two streams of
    // timestamps drift apart and skew staleness/expiry comparisons.
    let start = tokio::time::Instant::now();

    let mut tasks = JoinSet::new();
    tasks.spawn(named_task(
        "engine-actor",
        EngineActor::new(engine, adapter, start).run(engine_rx),
    ));

    tokio::select! {
        () = accept_loop(&endpoint, &engine_tx, &mut tasks, start) => {}
        result = shutdown_rx => {
            // A dropped sender (host caller gone without an explicit
            // shutdown) is treated the same as an explicit signal, so
            // `.ok()` discards the distinction intentionally.
            result.ok();
        }
    }
    info!("shutdown requested");
    dump_latency_summary(&engine_tx).await;
    drop(engine_tx);
    endpoint.close(
        wtransport::VarInt::from_u32(0),
        b"session host shutting down",
    );
    shutdown::join_with_timeout(tasks).await;
}

/// Requests and logs the engine actor's per-stage latency summary
/// (ADR-0009), best-effort: a reply that never arrives (actor already gone)
/// is not itself a shutdown failure.
async fn dump_latency_summary(engine_tx: &mpsc::Sender<ToEngine>) {
    let (reply_tx, reply_rx) = oneshot::channel();
    if engine_tx
        .send(ToEngine::DumpLatencySummary { reply: reply_tx })
        .await
        .is_err()
    {
        return;
    }
    if let Ok(summary) = reply_rx.await {
        info!(%summary, "latency summary at shutdown");
    }
}

/// Accepts incoming WebTransport sessions forever, spawning one connection
/// task per accepted session into `tasks` so shutdown can wait on and, if
/// needed, abort them alongside the engine actor.
async fn accept_loop(
    endpoint: &Endpoint<wtransport::endpoint::endpoint_side::Server>,
    engine_tx: &mpsc::Sender<ToEngine>,
    tasks: &mut JoinSet<()>,
    start: tokio::time::Instant,
) {
    let next_client = AtomicU64::new(0);
    loop {
        let incoming = endpoint.accept().await;
        // Harvest every connection task that finished since the last accept so
        // the JoinSet does not retain completed JoinHandles for the process
        // lifetime; without this the set grows unbounded with connection count
        // (ADR-0015). A panicked connection task is logged, not propagated: one
        // bad connection must not take down the accept loop.
        while let Some(joined) = tasks.try_join_next() {
            if let Err(error) = joined
                && !error.is_cancelled()
            {
                warn!(%error, "connection task panicked");
            }
        }
        let engine_tx = engine_tx.clone();
        let client = ClientKey::new(next_client.fetch_add(1, Ordering::Relaxed));
        tasks.spawn(named_task(
            "connection",
            accept_and_run(incoming, client, start, engine_tx),
        ));
    }
}

/// Completes one incoming session's handshake and drives its connection
/// task, logging (rather than propagating) a failed handshake since one
/// bad connection must not take down the accept loop.
async fn accept_and_run(
    incoming: wtransport::endpoint::IncomingSession,
    client: ClientKey,
    start: tokio::time::Instant,
    engine_tx: mpsc::Sender<ToEngine>,
) {
    let session_request = match incoming.await {
        Ok(request) => request,
        Err(error) => {
            warn!(%error, "incoming session request failed");
            return;
        }
    };
    let connection = match session_request.accept().await {
        Ok(connection) => connection,
        Err(error) => {
            warn!(%error, "session accept failed");
            return;
        }
    };
    connection::run_connection(connection, client, start, engine_tx).await;
}

/// Wraps a future so its exit is logged by name (ADR-0015: critical tasks
/// get a name, so the shutdown log's straggler report is legible).
async fn named_task(name: &'static str, future: impl std::future::Future<Output = ()>) {
    future.await;
    tracing::debug!(task = name, "task exited");
}
