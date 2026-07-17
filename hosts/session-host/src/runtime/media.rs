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
use tracing::{debug, warn};
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

/// Encodes `frame` to JPEG once and hands it to every client's writer for the
/// frame's source, lazily spawning that per-source writer on first use,
/// counting a drop for any source whose in-flight slot is still full (slow
/// consumer), and evicting any client whose connection has gone away.
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
    let mut closed = Vec::new();
    for (client, entry) in clients.iter_mut() {
        let sink = entry.sources.entry(source_id).or_insert_with(|| {
            let (frame_tx, frame_rx) = mpsc::channel::<EncodedFrame>(1);
            writers.spawn(client_writer(
                *client,
                source_id,
                entry.connection.clone(),
                frame_rx,
                start,
            ));
            ClientSink {
                frames: frame_tx,
                dropped: 0,
            }
        });
        match sink.frames.try_send(encoded.clone()) {
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

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::encode_jpeg;
    use crate::runtime::stream_tag::{FOURCC_MJPEG, VIDEO_FRAME_V2, frame_video_payload_v2};
    use pilotage_adapter_api::{
        CalibrationId, CameraId, CaptureClockMapping, MeasurementClock, MeasurementStamp,
        SourceIncarnation, SourceIntegrity, SourceRole, VideoCaptureStamp,
    };
    use pilotage_adapter_gazebo::RawVideoFrame;
    use pilotage_timing::SimTick;

    fn capture_stamp() -> VideoCaptureStamp {
        VideoCaptureStamp {
            stamp: MeasurementStamp {
                role: SourceRole::VideoCapture,
                integrity: SourceIntegrity::Unprotected,
                source_id: 1,
                source_incarnation: SourceIncarnation::new([5; 16]),
                source_epoch: 0,
                sequence: 3,
                acquired_at_ns: 999,
                clock: MeasurementClock::Simulation,
            },
            camera_id: CameraId(1),
            calibration_id: CalibrationId::NONE,
            mapping: CaptureClockMapping::identity(MeasurementClock::Simulation),
        }
    }

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
            capture: capture_stamp(),
        }
    }

    #[test]
    fn encodes_frame_and_v2_body_carries_the_jpeg() {
        let frame = synthetic_rgb(16, 12);
        let jpeg = encode_jpeg(&frame).expect("synthetic RGB frame encodes to JPEG");
        // A JPEG stream begins with the SOI marker 0xFFD8 and ends with EOI
        // 0xFFD9; check both so a garbage encode is caught.
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "JPEG starts with SOI");
        assert_eq!(&jpeg[jpeg.len() - 2..], &[0xFF, 0xD9], "JPEG ends with EOI");

        // Frame the JPEG exactly as the media task writes it after the tag: a
        // v2 capture-identity body (ADR-0020). The full on-wire unit leads with
        // the v2 kind tag, then the header, codec, length prefix, and payload.
        let body = frame_video_payload_v2(1, &frame.capture, 10, 20, FOURCC_MJPEG, &jpeg)
            .expect("JPEG frames into a v2 body");
        assert_eq!(body[0], 1, "leads with the chase source id");
        assert_eq!(
            &body[body.len() - jpeg.len()..],
            jpeg.as_slice(),
            "JPEG trails intact"
        );

        let mut wire = vec![VIDEO_FRAME_V2];
        wire.extend_from_slice(&body);
        assert_eq!(wire[0], VIDEO_FRAME_V2, "leads with the v2 video kind tag");
    }

    #[test]
    fn non_rgb_frame_is_skipped() {
        let mut frame = synthetic_rgb(4, 4);
        frame.pixel_format = "BGR_INT8".to_owned();
        assert!(encode_jpeg(&frame).is_none());
    }
}
