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
use pilotage_adapter_gazebo::RawVideoFrame;
use pilotage_session::ClientKey;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use tokio::task::JoinSet;
use tracing::{debug, error, warn};
use wtransport::Connection;

use crate::runtime::stream_tag::{FOURCC_MJPEG, VIDEO_FRAME, frame_video_payload};

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
    /// Registers `client`'s connection to receive video frames. Best-effort:
    /// a closed channel (media task gone) is ignored, since a host without a
    /// running media task simply serves no video.
    pub fn register(&self, client: ClientKey, connection: Connection) {
        self.commands
            .try_send(MediaCommand::Register(MediaClient { client, connection }))
            .ok();
    }

    /// Deregisters `client` on disconnect. Best-effort for the same reason as
    /// [`Self::register`].
    pub fn deregister(&self, client: ClientKey) {
        self.commands
            .try_send(MediaCommand::Deregister(client))
            .ok();
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
) -> (MediaHandle, tokio::task::JoinHandle<()>) {
    let (commands_tx, commands_rx) = mpsc::channel(COMMAND_QUEUE_CAPACITY);
    let handle = tokio::spawn(media_loop(frames, commands_rx));
    (
        MediaHandle {
            commands: commands_tx,
        },
        handle,
    )
}

/// Per-(client, source) outbound handoff of the latest encoded JPEG. Capacity
/// 1 bounds in-flight frames to one per source per client: `try_send` on a
/// full channel means that source's writer is still busy with its previous
/// frame, so the new frame is dropped-to-latest and counted (ADR-0011
/// best-effort media).
type FrameTx = mpsc::Sender<Arc<Vec<u8>>>;

/// The media task's own view of one video source of a connected client: the
/// handoff channel to that source's writer task, plus the running drop count
/// for frames its writer could not keep up with.
struct ClientSink {
    frames: FrameTx,
    dropped: u64,
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
                    Some(frame) => fan_out_frame(frame, &mut clients, &mut writers),
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

/// Encodes `frame` to JPEG once and hands it to every client's writer for the
/// frame's source, lazily spawning that per-source writer on first use,
/// counting a drop for any source whose in-flight slot is still full (slow
/// consumer), and evicting any client whose connection has gone away.
fn fan_out_frame(
    frame: RawVideoFrame,
    clients: &mut std::collections::BTreeMap<ClientKey, ClientEntry>,
    writers: &mut JoinSet<()>,
) {
    if clients.is_empty() {
        return;
    }
    let source_id = frame.source_id;
    let Some(jpeg) = encode_jpeg(&frame) else {
        return;
    };
    let jpeg = Arc::new(jpeg);
    let mut closed = Vec::new();
    for (client, entry) in clients.iter_mut() {
        let sink = entry.sources.entry(source_id).or_insert_with(|| {
            let (frame_tx, frame_rx) = mpsc::channel::<Arc<Vec<u8>>>(1);
            writers.spawn(client_writer(
                *client,
                source_id,
                entry.connection.clone(),
                frame_rx,
            ));
            ClientSink {
                frames: frame_tx,
                dropped: 0,
            }
        });
        match sink.frames.try_send(Arc::clone(&jpeg)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                sink.dropped = sink.dropped.wrapping_add(1);
                warn!(
                    client = client.as_u64(),
                    source_id,
                    total_dropped = sink.dropped,
                    "video frame dropped: client writer still busy with the previous frame"
                );
            }
            Err(TrySendError::Closed(_)) => closed.push(*client),
        }
    }
    for client in closed {
        clients.remove(&client);
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

/// Per-(client, source) writer: receives the latest encoded JPEG and writes it
/// as one host-initiated uni stream tagged [`VIDEO_FRAME`], one stream per
/// frame (ADR-0005), stamped with `source_id`. Exits when the handoff channel
/// closes (client deregistered or media task shutting down).
async fn client_writer(
    client: ClientKey,
    source_id: u8,
    connection: Connection,
    mut frames: mpsc::Receiver<Arc<Vec<u8>>>,
) {
    while let Some(jpeg) = frames.recv().await {
        if let Err(reason) = write_one_frame(&connection, source_id, &jpeg).await {
            // A failed uni-stream open/write means the connection is going
            // away; stop writing video to it. The connection task's own
            // teardown deregisters this client.
            debug!(
                client = client.as_u64(),
                source_id, reason, "video writer stopping"
            );
            return;
        }
    }
}

/// Opens one host-initiated uni stream, writes the [`VIDEO_FRAME`] tag then
/// the source-tagged, codec-tagged, length-prefixed JPEG, and finishes the
/// stream (ADR-0005 media unit; ADR-0016 FourCC codec tag).
async fn write_one_frame(
    connection: &Connection,
    source_id: u8,
    jpeg: &[u8],
) -> Result<(), &'static str> {
    let Some(body) = frame_video_payload(source_id, FOURCC_MJPEG, jpeg) else {
        // A frame larger than u32::MAX cannot be length-prefixed; skip it
        // without failing the writer (no real camera frame reaches this).
        error!("video frame exceeds u32 length prefix; skipping");
        return Ok(());
    };
    let mut stream = connection
        .open_uni()
        .await
        .map_err(|_| "uni stream request failed")?
        .await
        .map_err(|_| "uni stream failed to open")?;
    stream
        .write_all(&[VIDEO_FRAME])
        .await
        .map_err(|_| "tag write failed")?;
    stream
        .write_all(&body)
        .await
        .map_err(|_| "payload write failed")?;
    stream.finish().await.map_err(|_| "stream finish failed")?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::encode_jpeg;
    use crate::runtime::stream_tag::{
        FOURCC_MJPEG, VIDEO_FRAME, frame_video_payload, parse_video_payload,
    };
    use pilotage_adapter_gazebo::RawVideoFrame;
    use pilotage_timing::SimTick;

    /// Builds a synthetic RGB frame with a simple gradient so the encoder has
    /// real (non-constant) pixel data to work with.
    fn synthetic_rgb(width: u32, height: u32) -> RawVideoFrame {
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for y in 0..height {
            for x in 0..width {
                rgb.push((x % 256) as u8);
                rgb.push((y % 256) as u8);
                rgb.push(((x + y) % 256) as u8);
            }
        }
        RawVideoFrame {
            source_id: 0,
            width,
            height,
            pixel_format: "RGB_INT8".to_owned(),
            tick: SimTick::new(0),
            rgb,
        }
    }

    #[test]
    fn encodes_frames_and_wire_frames_round_trip() {
        let frame = synthetic_rgb(16, 12);
        let jpeg = encode_jpeg(&frame).expect("synthetic RGB frame encodes to JPEG");
        // A JPEG stream begins with the SOI marker 0xFFD8 and ends with EOI
        // 0xFFD9; check both so a garbage encode is caught.
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "JPEG starts with SOI");
        assert_eq!(&jpeg[jpeg.len() - 2..], &[0xFF, 0xD9], "JPEG ends with EOI");

        // Frame the JPEG exactly as the media task writes it after the tag,
        // then parse the source-tagged, codec-tagged length-prefixed body back
        // (ADR-0016).
        let body = frame_video_payload(1, FOURCC_MJPEG, &jpeg).expect("JPEG frames");
        let (source_id, codec, parsed) =
            parse_video_payload(&body).expect("framed body parses back");
        assert_eq!(source_id, 1, "carries the chase source id");
        assert_eq!(codec, FOURCC_MJPEG, "carries the MJPG FourCC");
        assert_eq!(parsed, jpeg.as_slice(), "round-trips the exact JPEG bytes");

        // The full on-wire unit is [tag][source_id][fourcc][u32 len][jpeg];
        // assemble and dissect it the way a client reader would.
        let mut wire = vec![VIDEO_FRAME];
        wire.extend_from_slice(&body);
        assert_eq!(wire[0], VIDEO_FRAME, "leads with the video kind tag");
        let (source_id, codec, parsed) =
            parse_video_payload(&wire[1..]).expect("payload after tag parses");
        assert_eq!(source_id, 1);
        assert_eq!(codec, FOURCC_MJPEG);
        assert_eq!(parsed, jpeg.as_slice());
    }

    #[test]
    fn non_rgb_frame_is_skipped() {
        let mut frame = synthetic_rgb(4, 4);
        frame.pixel_format = "BGR_INT8".to_owned();
        assert!(encode_jpeg(&frame).is_none());
    }
}
