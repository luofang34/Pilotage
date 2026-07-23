//! Host-side video media pipeline (ADR-0005 media = one uni stream per
//! frame; ADR-0011 best-effort with counted drops).
//!
//! A single media task drains the Gazebo adapter's raw RGB frame receiver,
//! encodes each frame to JPEG once, and fans the encoded frame out to every
//! connected client. Frames carry a `source_id` (0 = onboard FPV, 1 = chase);
//! each client gets a separate writer task and capacity-1 handoff channel
//! *per source*, so a client that drains one source slower than frames arrive
//! drops that source to its latest frame (the stale one is discarded and
//! counted) without stalling the other source or control/telemetry. Encode
//! happens once per frame regardless of client count; the JPEG is shared as an
//! `Arc`.

use std::sync::Arc;

use jpeg_encoder::{ColorType, Encoder};
use pilotage_adapter_api::VideoCaptureStamp;
use pilotage_adapter_gazebo::RawVideoFrame;
use pilotage_session::ClientKey;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use tokio::task::JoinSet;
use tokio::time::Instant;
use tracing::{debug, error, warn};
use wtransport::Connection;

use frame_writer::client_writer;

mod frame_writer;

/// JPEG quality (1-100) for encoded camera frames. 75 balances size against
/// visible quality for a teleop preview (ADR-0005: owned, tunable pipeline).
const JPEG_QUALITY: u8 = 75;

/// A client the media task should start sending video to.
struct MediaClient {
    client: ClientKey,
    connection: Connection,
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
    pub fn register(&self, client: ClientKey, connection: Connection) {
        if self
            .commands
            .try_send(MediaCommand::Register(MediaClient { client, connection }))
            .is_err()
        {
            warn!(
                client = client.as_u64(),
                "media register dropped (queue full or task gone); client will receive no video"
            );
        }
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

/// One encoded frame handed to a client's writer: the shared JPEG, the capture
/// identity to serialize alongside it, and the host receive time stamped when
/// the media task dequeued the raw frame (kept distinct from the capture time).
#[derive(Clone)]
struct EncodedFrame {
    jpeg: Arc<Vec<u8>>,
    capture: VideoCaptureStamp,
    received_at_ns: u64,
}

/// Per-(client, source) outbound handoff of the latest encoded frame. Capacity
/// 1 bounds in-flight frames to one per source per client: `try_send` on a
/// full channel means that source's writer is still busy with its previous
/// frame, so the new frame is dropped-to-latest and counted (ADR-0011
/// best-effort media).
type FrameTx = mpsc::Sender<EncodedFrame>;

/// Writer respawns allowed per (client, source) before the source is retired
/// for that client. A writer that exits while its connection still lives is a
/// transient (a stream-open error classified connection-fatal, a QUIC
/// hiccup); a genuinely dead connection's respawned writer exits again within
/// one frame, and connection teardown deregisters the whole client anyway —
/// the bound keeps a wedged connection from costing one writer spawn per
/// frame forever.
const MAX_WRITER_RESPAWNS: u32 = 3;

/// What fan-out does with a sink whose writer task has exited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SinkAction {
    /// Spawn a fresh writer (recording this exit) and hand it the frame.
    Respawn,
    /// The bound is exhausted: retire this source for this client, loudly.
    Retire,
}

/// Pure respawn policy: `writer_exits` counts exits BEFORE this one.
fn on_writer_exit(writer_exits: u32) -> SinkAction {
    if writer_exits < MAX_WRITER_RESPAWNS {
        SinkAction::Respawn
    } else {
        SinkAction::Retire
    }
}

/// The media task's own view of one video source of a connected client: the
/// handoff channel to that source's writer task, the running drop count for
/// frames its writer could not keep up with, and how many writers for this
/// source have exited. A retired sink keeps the source dark for this client
/// (other sources and the client itself live on; only connection teardown
/// removes the CLIENT).
enum ClientSink {
    /// A live writer task holds the other end of `frames`.
    Live {
        frames: FrameTx,
        dropped: u64,
        writer_exits: u32,
    },
    /// The writer exit bound is exhausted; the source stays dark, loudly.
    Retired,
}

/// The media task's view of a connected client: its connection (so per-source
/// writer tasks can be spawned lazily on the first frame seen for each source)
/// and the per-source sinks already opened.
struct ClientEntry {
    connection: Connection,
    sources: std::collections::BTreeMap<u8, ClientSink>,
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
                    Some(command) => apply_command(command, &mut clients),
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
) {
    match command {
        MediaCommand::Register(MediaClient { client, connection }) => {
            clients.insert(
                client,
                ClientEntry {
                    connection,
                    sources: std::collections::BTreeMap::new(),
                },
            );
        }
        MediaCommand::Deregister(client) => {
            clients.remove(&client);
        }
    }
}

/// One frame's fate at one client sink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SinkDelivery {
    /// Handed to the live writer.
    Delivered,
    /// The writer is still busy with the previous frame; carries the new
    /// running drop total.
    DroppedFull(u64),
    /// The writer had exited; a fresh one took the frame. Carries the new
    /// exit count.
    Respawned(u32),
    /// The writer exit bound is exhausted; the sink retired on this frame.
    Retired(u32),
    /// The sink was already retired; the source stays dark for this client.
    Skipped,
}

/// Routes one encoded frame into a sink, absorbing a dead writer through the
/// bounded respawn policy. `respawn(exits)` builds the replacement live sink
/// carrying the new exit count; it is injected so the transition is testable
/// with real channels and no live connection.
fn deliver_to_sink(
    sink: &mut ClientSink,
    encoded: &EncodedFrame,
    respawn: impl FnOnce(u32) -> ClientSink,
) -> SinkDelivery {
    let ClientSink::Live {
        frames,
        dropped,
        writer_exits,
    } = sink
    else {
        return SinkDelivery::Skipped;
    };
    match frames.try_send(encoded.clone()) {
        Ok(()) => SinkDelivery::Delivered,
        Err(TrySendError::Full(_)) => {
            *dropped = dropped.wrapping_add(1);
            SinkDelivery::DroppedFull(*dropped)
        }
        Err(TrySendError::Closed(_)) => {
            // The writer task exited (a connection-fatal classification). The
            // CLIENT is not evicted here — a live connection deserves a fresh
            // writer, and a dead one is deregistered by its connection task's
            // teardown (with the all-retired backstop below covering a lost
            // deregistration). Only this source's writer is respawned,
            // bounded, then retired.
            let exits = *writer_exits;
            match on_writer_exit(exits) {
                SinkAction::Respawn => {
                    *sink = respawn(exits + 1);
                    if let ClientSink::Live { frames, .. } = sink {
                        frames.try_send(encoded.clone()).ok();
                    }
                    SinkDelivery::Respawned(exits + 1)
                }
                SinkAction::Retire => {
                    *sink = ClientSink::Retired;
                    SinkDelivery::Retired(exits)
                }
            }
        }
    }
}

/// Whether every source of a client has retired — the media-side backstop
/// for a LOST deregistration: such a client receives nothing anyway, and a
/// genuinely dead connection's writers retire within a few frames, so
/// removing it restores the old eviction guarantee with bounded patience.
fn fully_retired(sources: &std::collections::BTreeMap<u8, ClientSink>) -> bool {
    !sources.is_empty()
        && sources
            .values()
            .all(|sink| matches!(sink, ClientSink::Retired))
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
        let ClientEntry {
            connection,
            sources,
        } = entry;
        let sink = sources
            .entry(source_id)
            .or_insert_with(|| spawn_sink(*client, source_id, connection, writers, start, 0));
        let delivery = deliver_to_sink(sink, &encoded, |exits| {
            spawn_sink(*client, source_id, connection, writers, start, exits)
        });
        match delivery {
            SinkDelivery::Delivered | SinkDelivery::Skipped => {}
            SinkDelivery::DroppedFull(total_dropped) => warn!(
                client = client.as_u64(),
                source_id,
                total_dropped,
                "video frame dropped: client writer still busy with the previous frame"
            ),
            SinkDelivery::Respawned(writer_exits) => warn!(
                client = client.as_u64(),
                source_id, writer_exits, "video writer exited; respawning for this source"
            ),
            SinkDelivery::Retired(writer_exits) => {
                error!(
                    client = client.as_u64(),
                    source_id,
                    writer_exits,
                    "video source retired for this client: writer exit bound exhausted"
                );
                if fully_retired(sources) {
                    reaped.push(*client);
                }
            }
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

/// Spawns one per-(client, source) writer task and returns its live sink,
/// carrying the exit count the respawn bound is judged against.
fn spawn_sink(
    client: ClientKey,
    source_id: u8,
    connection: &Connection,
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
        start,
    ));
    ClientSink::Live {
        frames: frame_tx,
        dropped: 0,
        writer_exits,
    }
}

/// Encodes one raw RGB frame to JPEG. Returns `None` (and logs) on a
/// non-RGB pixel format or an encoder failure, so the media task skips a bad
/// frame rather than tearing down the whole pipeline.
fn encode_jpeg(frame: &RawVideoFrame) -> Option<Vec<u8>> {
    if frame.pixel_format != "RGB_INT8" {
        warn!(
            format = frame.pixel_format,
            "skipping frame: only RGB_INT8 is supported by the media encoder"
        );
        return None;
    }
    let (width, height) = match (u16::try_from(frame.width), u16::try_from(frame.height)) {
        (Ok(w), Ok(h)) => (w, h),
        _ => {
            warn!(
                width = frame.width,
                height = frame.height,
                "skipping frame: dimensions exceed the JPEG encoder's 16-bit limit"
            );
            return None;
        }
    };
    let mut jpeg = Vec::new();
    let encoder = Encoder::new(&mut jpeg, JPEG_QUALITY);
    if let Err(error) = encoder.encode(&frame.rgb, width, height, ColorType::Rgb) {
        warn!(%error, "JPEG encode failed; skipping frame");
        return None;
    }
    Some(jpeg)
}

#[cfg(test)]
mod tests;
