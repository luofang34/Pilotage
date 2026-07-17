//! Per-(client, source) video frame writer: one host-initiated uni
//! stream per frame, written under a deadline so a stalled consumer
//! costs one frame rather than wedging its source permanently.

use std::future::Future;
use std::time::Duration;

use pilotage_session::ClientKey;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tracing::{debug, error, warn};
use wtransport::{Connection, SendStream, VarInt};

use super::{EncodedFrame, now_ns};
use crate::runtime::stream_tag::{FOURCC_MJPEG, VIDEO_FRAME_V2, frame_video_payload_v2};

/// Longest a single frame's uni-stream write may take before the stream
/// is reset and the writer moves on. A client that stops consuming one
/// stream (per-stream flow control fills) would otherwise park the write
/// forever: the capacity-1 handoff then never frees, every later frame is
/// dropped-to-latest, and that source is dead for that client until
/// reconnect — a wedged stream must cost one frame, not the source.
/// Generous against transient congestion (a healthy frame completes in
/// milliseconds on the deployment link).
const FRAME_WRITE_DEADLINE: Duration = Duration::from_secs(2);

/// Application error code carried on the RESET_STREAM of a
/// deadline-exceeded frame. Informational to the peer, which discards the
/// partial frame regardless of the code.
const STALL_RESET_CODE: u32 = 1;

/// One per-frame outbound stream. `write_all`/`finish` are the clean send
/// path; `reset` is the explicit RESET_STREAM a deadline-exceeded frame
/// needs. Dropping a wtransport/Quinn `SendStream` attempts a graceful
/// FIN, not a reset — a stalled peer never drains it, so truncated
/// streams would linger and eventually exhaust its stream allowance.
/// Resetting the retained stream frees its slot immediately. Send-bounded
/// because the writer runs as a spawned task on a multi-threaded runtime.
trait FrameStream {
    /// Writes the whole buffer, awaiting flow-control credit.
    fn write_all(&mut self, buf: &[u8]) -> impl Future<Output = Result<(), &'static str>> + Send;
    /// Finishes the stream cleanly (graceful FIN).
    fn finish(&mut self) -> impl Future<Output = Result<(), &'static str>> + Send;
    /// Resets the stream (RESET_STREAM); a no-op on an already-closed one.
    fn reset(&mut self);
}

/// Opens per-frame outbound streams on a connection.
trait FrameChannel {
    /// The stream this channel opens.
    type Stream: FrameStream;
    /// Opens one host-initiated uni stream.
    fn open(&self) -> impl Future<Output = Result<Self::Stream, &'static str>> + Send;
}

impl FrameStream for SendStream {
    async fn write_all(&mut self, buf: &[u8]) -> Result<(), &'static str> {
        SendStream::write_all(self, buf)
            .await
            .map_err(|_| "payload write failed")
    }

    async fn finish(&mut self) -> Result<(), &'static str> {
        SendStream::finish(self)
            .await
            .map_err(|_| "stream finish failed")
    }

    fn reset(&mut self) {
        // Best-effort: a stream already finished or reset returns
        // ClosedStream, which is nothing to act on here.
        SendStream::reset(self, VarInt::from_u32(STALL_RESET_CODE)).ok();
    }
}

impl FrameChannel for Connection {
    type Stream = SendStream;

    async fn open(&self) -> Result<SendStream, &'static str> {
        self.open_uni()
            .await
            .map_err(|_| "uni stream request failed")?
            .await
            .map_err(|_| "uni stream failed to open")
    }
}

/// Per-(client, source) writer: receives the latest encoded frame and writes
/// it as one host-initiated uni stream tagged [`VIDEO_FRAME_V2`], one stream
/// per frame (ADR-0005, ADR-0020), leading with the frame's capture identity.
/// Exits when the handoff channel closes (client deregistered or media task
/// shutting down).
pub(super) async fn client_writer(
    client: ClientKey,
    source_id: u8,
    connection: Connection,
    mut frames: mpsc::Receiver<EncodedFrame>,
    start: Instant,
) {
    drain_frames(client, source_id, &connection, &mut frames, start).await;
}

/// What one frame's delivery attempt produced.
enum FrameOutcome {
    /// Written and finished cleanly.
    Sent,
    /// The frame exceeded its deadline. `reset` is true when the stream
    /// had opened and was explicitly reset, false when the open itself
    /// timed out (there was no stream to reset).
    Stalled { reset: bool },
    /// The open or write failed outright; the connection is going away.
    Failed(&'static str),
}

/// Drains the handoff channel, delivering one frame per stream under a
/// single absolute per-frame deadline that covers BOTH opening the stream
/// and writing it. A frame that exceeds its deadline costs one frame and
/// the writer proceeds — a mid-write timeout resets the opened stream
/// (RESET_STREAM), an open timeout simply skips (no stream exists). An
/// outright open/write failure means the connection is going away and ends
/// the writer.
async fn drain_frames<C: FrameChannel>(
    client: ClientKey,
    source_id: u8,
    channel: &C,
    frames: &mut mpsc::Receiver<EncodedFrame>,
    start: Instant,
) {
    let mut stalls: u64 = 0;
    while let Some(frame) = frames.recv().await {
        // Stamp publication at the moment of write, distinct from the receive
        // stamp taken at dequeue, so a consumer can separate host queueing
        // latency from the capture-to-receipt gap.
        let published_at_ns = now_ns(start);
        let Some(body) = frame_video_payload_v2(
            source_id,
            &frame.capture,
            frame.received_at_ns,
            published_at_ns,
            FOURCC_MJPEG,
            &frame.jpeg,
        ) else {
            // A frame larger than u32::MAX cannot be length-prefixed; skip it
            // without failing the writer (no real camera frame reaches this).
            error!("video frame exceeds u32 length prefix; skipping");
            continue;
        };
        let deadline = Instant::now() + FRAME_WRITE_DEADLINE;
        match deliver_frame(channel, deadline, VIDEO_FRAME_V2, &body).await {
            FrameOutcome::Sent => {}
            FrameOutcome::Stalled { reset } => {
                stalls = stalls.wrapping_add(1);
                warn!(
                    client = client.as_u64(),
                    source_id,
                    total_stalls = stalls,
                    stream_reset = reset,
                    "video frame exceeded its deadline; continuing with the next frame"
                );
            }
            FrameOutcome::Failed(reason) => {
                // A failed open/write means the connection is going away; stop
                // writing video to it. The connection task's own teardown
                // deregisters this client.
                debug!(
                    client = client.as_u64(),
                    source_id, reason, "video writer stopping"
                );
                return;
            }
        }
    }
}

/// Opens a stream and writes the tag then the framed body, all under one
/// absolute `deadline`. An open that exceeds the deadline yields a stall
/// with no stream to reset; a write that exceeds it explicitly resets the
/// opened stream (RESET_STREAM), retained across the timed region so the
/// reset can fire rather than a dropped-future FIN.
async fn deliver_frame<C: FrameChannel>(
    channel: &C,
    deadline: Instant,
    tag: u8,
    body: &[u8],
) -> FrameOutcome {
    let mut stream = match tokio::time::timeout_at(deadline, channel.open()).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(reason)) => return FrameOutcome::Failed(reason),
        Err(_elapsed) => return FrameOutcome::Stalled { reset: false },
    };
    match tokio::time::timeout_at(deadline, write_body(&mut stream, tag, body)).await {
        Ok(Ok(())) => FrameOutcome::Sent,
        Ok(Err(reason)) => FrameOutcome::Failed(reason),
        Err(_elapsed) => {
            stream.reset();
            FrameOutcome::Stalled { reset: true }
        }
    }
}

/// Writes the one-byte tag, then the body, then finishes the stream.
async fn write_body<S: FrameStream>(
    stream: &mut S,
    tag: u8,
    body: &[u8],
) -> Result<(), &'static str> {
    stream.write_all(&[tag]).await?;
    stream.write_all(body).await?;
    stream.finish().await
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use pilotage_adapter_api::{
        CalibrationId, CameraId, CaptureClockMapping, MeasurementClock, MeasurementStamp,
        SourceIncarnation, SourceIntegrity, SourceRole, VideoCaptureStamp,
    };
    use pilotage_session::ClientKey;
    use tokio::sync::mpsc;
    use tokio::time::Instant;

    use super::{EncodedFrame, FrameChannel, FrameStream, drain_frames};

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

    fn encoded_frame() -> EncodedFrame {
        EncodedFrame {
            jpeg: Arc::new(vec![0xFF, 0xD8, 0xFF, 0xD9]),
            capture: capture_stamp(),
            received_at_ns: 0,
        }
    }

    /// Shared call tallies a test asserts against.
    #[derive(Default)]
    struct Tally {
        opened: AtomicU32,
        finished: AtomicU32,
        reset: AtomicU32,
    }

    /// How a mock stream behaves once opened.
    #[derive(Clone, Copy)]
    enum Write {
        /// `write_all` never completes (a wedged consumer).
        Stall,
        /// `write_all` returns an error (connection going away).
        Fail,
        /// Writes and finishes normally.
        Ok,
    }

    /// How a mock `open()` behaves.
    #[derive(Clone, Copy)]
    enum Open {
        /// `open()` never completes (the peer's stream allowance is full).
        Stall,
        /// `open()` yields a stream with the given write behavior.
        Ready(Write),
    }

    struct MockStream {
        write: Write,
        tally: Arc<Tally>,
    }

    impl FrameStream for MockStream {
        async fn write_all(&mut self, _buf: &[u8]) -> Result<(), &'static str> {
            match self.write {
                Write::Stall => {
                    std::future::pending::<()>().await;
                    Ok(())
                }
                Write::Fail => Err("payload write failed"),
                Write::Ok => Ok(()),
            }
        }

        async fn finish(&mut self) -> Result<(), &'static str> {
            self.tally.finished.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn reset(&mut self) {
            self.tally.reset.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Hands out opens from a per-open script; opens beyond the script are
    /// `Ready(Write::Ok)`.
    struct MockChannel {
        script: Vec<Open>,
        tally: Arc<Tally>,
    }

    impl FrameChannel for MockChannel {
        type Stream = MockStream;

        async fn open(&self) -> Result<MockStream, &'static str> {
            let n = self.tally.opened.fetch_add(1, Ordering::SeqCst) as usize;
            match self
                .script
                .get(n)
                .copied()
                .unwrap_or(Open::Ready(Write::Ok))
            {
                Open::Stall => {
                    std::future::pending::<()>().await;
                    unreachable!("a stalled open never resolves")
                }
                Open::Ready(write) => Ok(MockStream {
                    write,
                    tally: self.tally.clone(),
                }),
            }
        }
    }

    async fn queue(frames: usize) -> mpsc::Receiver<EncodedFrame> {
        let (tx, rx) = mpsc::channel(frames.max(1));
        for _ in 0..frames {
            tx.send(encoded_frame()).await.expect("frame queues");
        }
        drop(tx);
        rx
    }

    /// A stalled write must reset its stream exactly once (RESET_STREAM,
    /// not a dropped FIN) and the next frame must open its own stream and
    /// finish cleanly. Virtual time fires the deadline without real waiting.
    #[tokio::test(start_paused = true)]
    async fn a_stalled_write_resets_once_and_the_next_frame_proceeds() {
        let mut rx = queue(2).await;
        let tally = Arc::new(Tally::default());
        let channel = MockChannel {
            script: vec![Open::Ready(Write::Stall), Open::Ready(Write::Ok)],
            tally: tally.clone(),
        };

        drain_frames(ClientKey::new(1), 0, &channel, &mut rx, Instant::now()).await;

        assert_eq!(
            tally.reset.load(Ordering::SeqCst),
            1,
            "the stalled frame's stream is reset exactly once"
        );
        assert_eq!(
            tally.opened.load(Ordering::SeqCst),
            2,
            "the next frame opens its own stream"
        );
        assert_eq!(
            tally.finished.load(Ordering::SeqCst),
            1,
            "the second frame finishes cleanly and is not reset"
        );
    }

    /// A stalled OPEN also costs one frame: the per-frame deadline covers
    /// opening, so an open that never completes is skipped (nothing to
    /// reset) and the next frame opens its own stream and finishes.
    #[tokio::test(start_paused = true)]
    async fn a_stalled_open_is_skipped_and_the_next_frame_proceeds() {
        let mut rx = queue(2).await;
        let tally = Arc::new(Tally::default());
        let channel = MockChannel {
            script: vec![Open::Stall, Open::Ready(Write::Ok)],
            tally: tally.clone(),
        };

        drain_frames(ClientKey::new(1), 0, &channel, &mut rx, Instant::now()).await;

        assert_eq!(
            tally.opened.load(Ordering::SeqCst),
            2,
            "the next frame opens its own stream after the stalled open"
        );
        assert_eq!(
            tally.finished.load(Ordering::SeqCst),
            1,
            "the second frame finishes cleanly"
        );
        assert_eq!(
            tally.reset.load(Ordering::SeqCst),
            0,
            "a stalled open has no stream to reset"
        );
    }

    /// An outright open/write error ends the writer: the connection is going
    /// away, so the frame after the failed one is never opened.
    #[tokio::test(start_paused = true)]
    async fn a_write_error_ends_the_writer() {
        let mut rx = queue(2).await;
        let tally = Arc::new(Tally::default());
        let channel = MockChannel {
            script: vec![Open::Ready(Write::Fail), Open::Ready(Write::Ok)],
            tally: tally.clone(),
        };

        drain_frames(ClientKey::new(1), 0, &channel, &mut rx, Instant::now()).await;

        assert_eq!(
            tally.opened.load(Ordering::SeqCst),
            1,
            "the writer stops after the failed frame; the next is never opened"
        );
        assert_eq!(
            tally.reset.load(Ordering::SeqCst),
            0,
            "an outright failure is not a reset"
        );
    }
}
