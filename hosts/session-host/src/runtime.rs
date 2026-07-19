//! Top-level orchestration: builds the WebTransport endpoint, the embedded
//! [`SessionEngine`]/adapter, and wires accept, engine-actor, and shutdown
//! tasks together (ADR-0002, ADR-0005).
//!
//! [`SessionEngine`]: pilotage_session::SessionEngine

mod aviate_profile;
mod connection;
mod engine_actor;
mod gazebo_launch;
mod media;
mod registry;
mod shutdown;
mod stream_tag;
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

use crate::cli::AdapterKind;
use crate::error::HostError;
use crate::output::print_line;
use crate::tls_identity::build_dev_identity;
use engine_actor::{ENGINE_QUEUE_CAPACITY, EngineActor, ToEngine};
use media::MediaHandle;

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
fn build_reference() -> (SessionEngine, ReferenceAdapter) {
    let adapter = ReferenceAdapter::from_seed(HOST_VEHICLE, ADAPTER_SEED);
    let engine = build_engine(&adapter);
    (engine, adapter)
}

/// Builds the session engine from an adapter's advertised capabilities and
/// this host's staleness/config policy, shared by every adapter path.
fn build_engine<A: VehicleAdapter>(adapter: &A) -> SessionEngine {
    let capabilities = adapter.capabilities();
    let staleness = StalenessPolicy::new(MAX_CONTROL_AGE);
    let config = SessionConfig::new(pilotage_protocol::SCHEMA_VERSION, env!("CARGO_PKG_VERSION"));
    SessionEngine::new(capabilities, staleness, config)
}

/// Starts the session host bound to `127.0.0.1:port` (`0` for an OS-assigned
/// ephemeral port), prints the `LISTENING` line, and returns a handle for
/// shutdown.
///
/// `adapter` selects the embedded vehicle adapter: [`AdapterKind::Reference`]
/// (default, 1a behavior, no video) or [`AdapterKind::Gazebo`] (real Gazebo
/// diff-drive through the sidecar bridge, with an MJPEG video downlink).
///
/// # Errors
///
/// Returns [`HostError`] if the TLS identity cannot be built, the
/// WebTransport endpoint cannot bind, or the Gazebo sidecar bridge cannot be
/// spawned and connected.
pub async fn start(port: u16, adapter: AdapterKind) -> Result<RunningHost, HostError> {
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

    let (engine_tx, engine_rx) = mpsc::channel::<ToEngine>(ENGINE_QUEUE_CAPACITY);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let join = spawn_host_runtime(adapter, endpoint, engine_tx, engine_rx, shutdown_rx).await?;

    Ok(RunningHost {
        local_addr,
        cert_hash_hex,
        shutdown_tx: Some(shutdown_tx),
        join,
    })
}

/// Builds the chosen adapter (and, for Gazebo, its media task) and spawns the
/// per-adapter `run_until_shutdown` task.
async fn spawn_host_runtime(
    adapter: AdapterKind,
    endpoint: Endpoint<wtransport::endpoint::endpoint_side::Server>,
    engine_tx: mpsc::Sender<ToEngine>,
    engine_rx: mpsc::Receiver<ToEngine>,
    shutdown_rx: oneshot::Receiver<()>,
) -> Result<tokio::task::JoinHandle<()>, HostError> {
    // A single monotonic origin feeds every `host_time` stamp across the host
    // (ADR-0009): the engine actor, each connection task, and — so a video
    // frame's receive/publication stamps stay comparable with them — the media
    // task all derive from this `Instant`.
    let start = tokio::time::Instant::now();
    match adapter {
        AdapterKind::Reference => {
            let (engine, adapter) = build_reference();
            Ok(tokio::spawn(run_until_shutdown(
                endpoint,
                engine,
                adapter,
                None,
                engine_tx,
                engine_rx,
                shutdown_rx,
                start,
            )))
        }
        AdapterKind::Gazebo => {
            let (engine, adapter, frames) =
                gazebo_launch::build_gazebo(HOST_VEHICLE, MAX_CONTROL_AGE).await?;
            let (media, media_task) = media::spawn_media_task(frames, start);
            Ok(tokio::spawn(run_with_media_until_shutdown(
                endpoint,
                engine,
                adapter,
                media,
                media_task,
                engine_tx,
                engine_rx,
                shutdown_rx,
                start,
            )))
        }
        AdapterKind::Aviate => {
            spawn_aviate_runtime(endpoint, engine_tx, engine_rx, shutdown_rx, start).await
        }
        AdapterKind::Px4 => {
            let mut adapter = pilotage_adapter_px4::Px4Adapter::start(HOST_VEHICLE)
                .await
                .map_err(HostError::Px4Adapter)?;
            let engine = build_engine(&adapter);
            match adapter.subscribe_frames() {
                Some(frames) => {
                    let (media, media_task) = media::spawn_media_task(frames, start);
                    Ok(tokio::spawn(run_with_media_until_shutdown(
                        endpoint,
                        engine,
                        adapter,
                        media,
                        media_task,
                        engine_tx,
                        engine_rx,
                        shutdown_rx,
                        start,
                    )))
                }
                None => Ok(tokio::spawn(run_until_shutdown(
                    endpoint,
                    engine,
                    adapter,
                    None,
                    engine_tx,
                    engine_rx,
                    shutdown_rx,
                    start,
                ))),
            }
        }
    }
}

/// Builds the Aviate adapter and spawns its runtime, wiring the media task only
/// when the adapter exposes a video frame source.
async fn spawn_aviate_runtime(
    endpoint: Endpoint<wtransport::endpoint::endpoint_side::Server>,
    engine_tx: mpsc::Sender<ToEngine>,
    engine_rx: mpsc::Receiver<ToEngine>,
    shutdown_rx: oneshot::Receiver<()>,
    start: tokio::time::Instant,
) -> Result<tokio::task::JoinHandle<()>, HostError> {
    // PILOTAGE_AVIATE_PROFILE selects the session profile (LINK-04):
    // "physical" (FC estimate + FC state; no truth), the default
    // "simulation" (estimate + FC state, plus the truth oracle when the
    // co-located shm block attaches), or "oracle-only" (truth stream
    // only; no uplink, no operational control). Parsing fails closed and
    // Physical gets the conservative link configuration.
    let profile = aviate_profile::profile_from_env(std::env::var("PILOTAGE_AVIATE_PROFILE"))?;
    let mut adapter = pilotage_adapter_aviate::AviateAdapter::start(
        HOST_VEHICLE,
        profile,
        aviate_profile::link_config(profile),
    )
    .await
    .map_err(HostError::AviateAdapter)?;
    let engine = build_engine(&adapter);
    match adapter.subscribe_frames() {
        Some(frames) => {
            let (media, media_task) = media::spawn_media_task(frames, start);
            Ok(tokio::spawn(run_with_media_until_shutdown(
                endpoint,
                engine,
                adapter,
                media,
                media_task,
                engine_tx,
                engine_rx,
                shutdown_rx,
                start,
            )))
        }
        None => Ok(tokio::spawn(run_until_shutdown(
            endpoint,
            engine,
            adapter,
            None,
            engine_tx,
            engine_rx,
            shutdown_rx,
            start,
        ))),
    }
}

/// Runs the accept loop and engine actor until `shutdown_rx` fires, then
/// tears down every spawned task (engine actor, accept loop, and every
/// per-connection task) within the shutdown timeout. `media` is `Some` only
/// for the Gazebo path, wiring each connection into the video downlink.
#[allow(clippy::too_many_arguments)]
async fn run_until_shutdown<A>(
    endpoint: Endpoint<wtransport::endpoint::endpoint_side::Server>,
    engine: SessionEngine,
    adapter: A,
    media: Option<MediaHandle>,
    engine_tx: mpsc::Sender<ToEngine>,
    engine_rx: mpsc::Receiver<ToEngine>,
    shutdown_rx: oneshot::Receiver<()>,
    start: tokio::time::Instant,
) where
    A: VehicleAdapter + Send + 'static,
{
    // `start` is the single monotonic origin every `host_time` stamp across
    // the host derives from (ADR-0009): client-message receive stamps (via
    // `accept_loop` and each connection task), the engine actor's
    // tick/offer-expiry stamps, and the media task's frame stamps. Sharing one
    // origin keeps those timestamp streams comparable rather than drifting.

    let mut tasks = JoinSet::new();
    tasks.spawn(named_task(
        "engine-actor",
        EngineActor::new(engine, adapter, start).run(engine_rx),
    ));

    tokio::select! {
        () = accept_loop(&endpoint, &engine_tx, &mut tasks, start, media.clone()) => {}
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

/// The camera-equipped variant of [`run_until_shutdown`]: identical
/// lifecycle, plus it joins the media task on the way out so no video uni
/// stream is abandoned mid-write. The media task ends once its frame
/// source (the adapter) is dropped, which the engine-actor task does at
/// teardown.
#[allow(clippy::too_many_arguments)]
async fn run_with_media_until_shutdown<A>(
    endpoint: Endpoint<wtransport::endpoint::endpoint_side::Server>,
    engine: SessionEngine,
    adapter: A,
    media: MediaHandle,
    media_task: tokio::task::JoinHandle<()>,
    engine_tx: mpsc::Sender<ToEngine>,
    engine_rx: mpsc::Receiver<ToEngine>,
    shutdown_rx: oneshot::Receiver<()>,
    start: tokio::time::Instant,
) where
    A: VehicleAdapter + Send + 'static,
{
    run_until_shutdown(
        endpoint,
        engine,
        adapter,
        Some(media),
        engine_tx,
        engine_rx,
        shutdown_rx,
        start,
    )
    .await;
    // The engine actor (owner of the adapter and its frame sender) has now
    // been torn down, closing the media task's frame source; wait for it to
    // drain and finish its writers.
    media_task.await.ok();
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
    media: Option<MediaHandle>,
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
            accept_and_run(incoming, client, start, engine_tx, media.clone()),
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
    media: Option<MediaHandle>,
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
    connection::run_connection(connection, client, start, engine_tx, media).await;
}

/// Wraps a future so its exit is logged by name (ADR-0015: critical tasks
/// get a name, so the shutdown log's straggler report is legible).
async fn named_task(name: &'static str, future: impl std::future::Future<Output = ()>) {
    future.await;
    tracing::debug!(task = name, "task exited");
}
