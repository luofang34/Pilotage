//! Host-side video media pipeline (ADR-0005 media = one uni stream per
//! frame; ADR-0011 best-effort with counted drops).
//!
//! A single media task drains the Gazebo adapter's raw RGB frame receiver,
//! encodes each frame to JPEG once, and fans the encoded frame out to every
//! connected client. Frames carry a source identity;
//! each client gets a separate writer task and capacity-1 handoff channel
//! *per source*, so a client that drains one source slower than frames arrive
//! drops that source to its latest frame (the stale one is discarded and
//! counted) without stalling the other source or control/telemetry. Encode
//! happens once per frame regardless of client count. A per-client aggregate
//! token bucket sheds video before it can crowd telemetry/control out of the
//! shared QUIC socket.

use std::sync::Arc;

use pilotage_adapter_gazebo::RawVideoFrame;
use pilotage_protocol::wire;
use pilotage_session::ClientKey;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;
use tokio::time::Instant;
use tracing::{debug, error, info, warn};
use wtransport::Connection;

use budget::{DeliveryMode, DeliveryState, PressureSignals, SendBudget, TransportSnapshot};
use encoding::{EncodedFrame, encode_jpeg};
use frame_writer::client_writer;
use sink::{ClientSink, SinkDelivery, deliver_to_sink, fully_retired};
#[cfg(test)]
use sink::{MAX_WRITER_RESPAWNS, SinkAction, on_writer_exit};

mod budget;
mod encoding;
mod frame_writer;
mod sink;

/// A client the media task should start sending video to.
struct MediaClient {
    client: ClientKey,
    connection: Connection,
    status: watch::Sender<wire::VideoDeliveryState>,
}

/// One command the media task accepts from connection tasks.
enum MediaCommand {
    /// Register a client so the next encoded frame is sent to it.
    Register(MediaClient),
    /// Deregister a client whose connection task has exited.
    Deregister(ClientKey),
}

/// Handle connection tasks use to register/deregister with the media task.
///
/// Cloneable and cheap; only the Gazebo adapter path constructs one. Dropping
/// every clone closes the command channel and lets the media task drain its
/// last frame and exit.
#[derive(Clone)]
pub struct MediaHandle {
    commands: mpsc::Sender<MediaCommand>,
}

impl MediaHandle {
    /// Registers `client`'s connection to receive video frames. Best-effort —
    /// a missing media task simply serves no video — but a DROPPED command is
    /// logged: an unregistered client silently gets no video otherwise.
    pub fn register(
        &self,
        client: ClientKey,
        connection: Connection,
    ) -> watch::Receiver<wire::VideoDeliveryState> {
        let (status, receiver) = watch::channel(wire_delivery_state(DeliveryState {
            mode: DeliveryMode::Normal,
            bytes_per_second: budget::MAX_BYTES_PER_SECOND,
        }));
        if self
            .commands
            .try_send(MediaCommand::Register(MediaClient {
                client,
                connection,
                status,
            }))
            .is_err()
        {
            warn!(
                client = client.as_u64(),
                "media register dropped (queue full or task gone); client will receive no video"
            );
        }
        receiver
    }

    /// Deregisters `client` on disconnect. Best-effort for the same reason as
    /// [`Self::register`]; a dropped command is logged, and the fan-out's
    /// all-sources-retired backstop reaps the client regardless.
    pub fn deregister(&self, client: ClientKey) {
        if self
            .commands
            .try_send(MediaCommand::Deregister(client))
            .is_err()
        {
            warn!(
                client = client.as_u64(),
                "media deregister dropped (queue full or task gone); the all-retired backstop reaps it"
            );
        }
    }
}

/// Capacity of the media task's command queue. Registrations and
/// deregistrations are infrequent (one per connection lifecycle), so a small
/// bound is ample.
const COMMAND_QUEUE_CAPACITY: usize = 256;

/// Spawns the media task and returns a [`MediaHandle`] connection tasks use to
/// register. The task drains `frames` until it closes (adapter gone), then
/// exits after its per-client writers finish.
pub fn spawn_media_task(
    frames: mpsc::Receiver<RawVideoFrame>,
    start: Instant,
) -> (MediaHandle, tokio::task::JoinHandle<()>) {
    let (commands_tx, commands_rx) = mpsc::channel(COMMAND_QUEUE_CAPACITY);
    let handle = tokio::spawn(media_loop(frames, commands_rx, start));
    (
        MediaHandle {
            commands: commands_tx,
        },
        handle,
    )
}

/// Host monotonic nanoseconds since `start`, saturating rather than
/// overflowing, in the same `host_time` domain the engine and connection tasks
/// stamp with (ADR-0009). Used for the receive/publication stamps that a
/// consumer must never confuse with a frame's capture time.
fn now_ns(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// The media task's view of a connected client: its connection (so per-source
/// writer tasks can be spawned lazily on the first frame seen for each source)
/// and the per-source sinks already opened.
struct ClientEntry {
    connection: Connection,
    sources: std::collections::BTreeMap<u8, ClientSink>,
    budget: SendBudget,
    pressure: Arc<PressureSignals>,
    transport: TransportSnapshot,
    status: watch::Sender<wire::VideoDeliveryState>,
}

/// Drains raw frames, encodes each once, and fans the encoded frame out to
/// every registered client, servicing register/deregister commands between
/// frames.
async fn media_loop(
    mut frames: mpsc::Receiver<RawVideoFrame>,
    mut commands: mpsc::Receiver<MediaCommand>,
    start: Instant,
) {
    let mut clients: std::collections::BTreeMap<ClientKey, ClientEntry> =
        std::collections::BTreeMap::new();
    let mut writers = JoinSet::new();
    loop {
        // Harvest every writer task that finished since the last iteration so
        // the JoinSet does not retain completed JoinHandles for the task
        // lifetime; without this the set grows unbounded under connection churn
        // on the video path (ADR-0015). A panicked writer is logged, not
        // propagated: one bad client must not tear down the media pipeline.
        while let Some(joined) = writers.try_join_next() {
            if let Err(error) = joined
                && !error.is_cancelled()
            {
                warn!(%error, "video writer task panicked");
            }
        }
        tokio::select! {
            command = commands.recv() => {
                match command {
                    Some(command) => apply_command(command, &mut clients, now_ns(start)),
                    None => break,
                }
            }
            frame = frames.recv() => {
                match frame {
                    // Stamp host receipt the moment the frame leaves the adapter
                    // channel, before the encode cost, so the receive time
                    // reflects arrival rather than processing.
                    Some(frame) => fan_out_frame(frame, now_ns(start), &mut clients, &mut writers, start),
                    None => break,
                }
            }
        }
    }
    // Dropping every client (and its sinks) closes each writer's handoff
    // channel, so the writers finish their in-flight frame and exit; wait for
    // them so no uni stream is abandoned mid-write on shutdown.
    drop(clients);
    while writers.join_next().await.is_some() {}
    debug!("media task exited");
}

/// Applies one register/deregister command. Registration only records the
/// client's connection; a per-source writer task is spawned lazily the first
/// time a frame for that source is fanned out (see [`fan_out_frame`]), so a
/// client is not charged a writer for a source that never streams.
fn apply_command(
    command: MediaCommand,
    clients: &mut std::collections::BTreeMap<ClientKey, ClientEntry>,
    now_ns: u64,
) {
    match command {
        MediaCommand::Register(MediaClient {
            client,
            connection,
            status,
        }) => {
            if let Some(entry) = clients.get_mut(&client) {
                entry.connection = connection;
                entry.sources.clear();
            } else {
                let transport = transport_snapshot(&connection);
                clients.insert(
                    client,
                    ClientEntry {
                        connection,
                        sources: std::collections::BTreeMap::new(),
                        budget: SendBudget::new(now_ns, transport),
                        pressure: Arc::new(PressureSignals::default()),
                        transport,
                        status,
                    },
                );
            }
        }
        MediaCommand::Deregister(client) => {
            clients.remove(&client);
        }
    }
}

/// Encodes `frame` to JPEG once and hands it to every client's writer for the
/// frame's source, lazily spawning that per-source writer on first use,
/// counting a drop for any source whose in-flight slot is still full (slow
/// consumer), respawning (bounded) any writer that exited, and removing a
/// client only once EVERY one of its sources has retired.
fn fan_out_frame(
    frame: RawVideoFrame,
    received_at_ns: u64,
    clients: &mut std::collections::BTreeMap<ClientKey, ClientEntry>,
    writers: &mut JoinSet<()>,
    start: Instant,
) {
    if clients.is_empty() {
        return;
    }
    let source_id = frame.source_id;
    let capture = frame.capture;
    let Some(jpeg) = encode_jpeg(&frame) else {
        return;
    };
    let encoded = EncodedFrame {
        jpeg: Arc::new(jpeg),
        capture,
        received_at_ns,
    };
    let mut reaped = Vec::new();
    for (client, entry) in clients.iter_mut() {
        if deliver_to_client(
            *client,
            source_id,
            &encoded,
            received_at_ns,
            entry,
            writers,
            start,
        ) {
            reaped.push(*client);
        }
    }
    for client in reaped {
        clients.remove(&client);
        warn!(
            client = client.as_u64(),
            "video client removed: every source retired (lost-deregistration backstop)"
        );
    }
}

fn deliver_to_client(
    client: ClientKey,
    source_id: u8,
    encoded: &EncodedFrame,
    now_ns: u64,
    entry: &mut ClientEntry,
    writers: &mut JoinSet<()>,
    start: Instant,
) -> bool {
    if entry.budget.feedback_due(now_ns) {
        entry.transport = transport_snapshot(&entry.connection);
    }
    let admission = entry.budget.admit(
        now_ns,
        source_id,
        encoded.jpeg.len(),
        entry.pressure.snapshot(),
        entry.transport,
    );
    if let Some(transition) = admission.transition {
        publish_transition(client, transition, &entry.status);
    }
    if !admission.admitted {
        return false;
    }
    let connection = &entry.connection;
    let pressure = Arc::clone(&entry.pressure);
    let sink = entry.sources.entry(source_id).or_insert_with(|| {
        spawn_sink_parts(
            client,
            source_id,
            connection,
            Arc::clone(&pressure),
            writers,
            start,
            0,
        )
    });
    let delivery = deliver_to_sink(sink, encoded, |exits| {
        spawn_sink_parts(
            client,
            source_id,
            connection,
            Arc::clone(&pressure),
            writers,
            start,
            exits,
        )
    });
    record_delivery(client, source_id, delivery, &entry.pressure);
    matches!(delivery, SinkDelivery::Retired(_)) && fully_retired(&entry.sources)
}

fn record_delivery(
    client: ClientKey,
    source_id: u8,
    delivery: SinkDelivery,
    pressure: &PressureSignals,
) {
    match delivery {
        SinkDelivery::Delivered | SinkDelivery::Skipped => {}
        SinkDelivery::DroppedFull(total_dropped) => {
            pressure.record_busy_drop();
            warn!(
                client = client.as_u64(),
                source_id,
                total_dropped,
                "video frame dropped: client writer still busy with the previous frame"
            );
        }
        SinkDelivery::Respawned(writer_exits) => warn!(
            client = client.as_u64(),
            source_id, writer_exits, "video writer exited; respawning for this source"
        ),
        SinkDelivery::Retired(writer_exits) => error!(
            client = client.as_u64(),
            source_id,
            writer_exits,
            "video source retired for this client: writer exit bound exhausted"
        ),
    }
}

fn publish_transition(
    client: ClientKey,
    state: DeliveryState,
    status: &watch::Sender<wire::VideoDeliveryState>,
) {
    status.send(wire_delivery_state(state)).ok();
    info!(
        client = client.as_u64(),
        mode = delivery_mode_label(state.mode),
        budget_bytes_per_second = state.bytes_per_second,
        reason = "bandwidth",
        "video delivery state changed"
    );
}

/// Spawns one per-(client, source) writer task and returns its live sink.
fn spawn_sink_parts(
    client: ClientKey,
    source_id: u8,
    connection: &Connection,
    pressure: Arc<PressureSignals>,
    writers: &mut JoinSet<()>,
    start: Instant,
    writer_exits: u32,
) -> ClientSink {
    let (frame_tx, frame_rx) = mpsc::channel::<EncodedFrame>(1);
    writers.spawn(client_writer(
        client,
        source_id,
        connection.clone(),
        frame_rx,
        pressure,
        start,
    ));
    ClientSink::Live {
        frames: frame_tx,
        dropped: 0,
        writer_exits,
    }
}

fn transport_snapshot(connection: &Connection) -> TransportSnapshot {
    let path = connection.quic_connection().stats().path;
    TransportSnapshot {
        lost_packets: path.lost_packets,
        congestion_events: path.congestion_events,
    }
}

fn wire_delivery_state(state: DeliveryState) -> wire::VideoDeliveryState {
    wire::VideoDeliveryState {
        mode: match state.mode {
            DeliveryMode::Normal => wire::VideoDeliveryMode::Normal.into(),
            DeliveryMode::Degraded => wire::VideoDeliveryMode::Degraded.into(),
            DeliveryMode::Suspended => wire::VideoDeliveryMode::Suspended.into(),
        },
        reason: match state.mode {
            DeliveryMode::Normal => wire::VideoDeliveryReason::Unspecified.into(),
            DeliveryMode::Degraded | DeliveryMode::Suspended => {
                wire::VideoDeliveryReason::Bandwidth.into()
            }
        },
        budget_bytes_per_second: state.bytes_per_second,
    }
}

fn delivery_mode_label(mode: DeliveryMode) -> &'static str {
    match mode {
        DeliveryMode::Normal => "normal",
        DeliveryMode::Degraded => "degraded",
        DeliveryMode::Suspended => "suspended",
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod transport_tests;
