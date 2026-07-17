//! Per-(client, source) video frame writer: one host-initiated uni
//! stream per frame, written under a deadline so a stalled consumer
//! costs one frame rather than wedging its source permanently.

use std::future::Future;
use std::time::Duration;

use pilotage_session::ClientKey;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tracing::{debug, error, warn};
use wtransport::Connection;

use super::{EncodedFrame, now_ns};
use crate::runtime::stream_tag::{FOURCC_MJPEG, VIDEO_FRAME_V2, frame_video_payload_v2};

/// Longest a single frame's uni-stream write may take before the stream
/// is abandoned and the writer moves on. A client that stops consuming
/// one stream (per-stream flow control fills) would otherwise park the
/// write forever: the capacity-1 handoff then never frees, every later
/// frame is dropped-to-latest, and that source is dead for that client
/// until reconnect — a wedged stream must cost one frame, not the
/// source. Generous against transient congestion (a healthy frame
/// completes in milliseconds on the deployment link).
const FRAME_WRITE_DEADLINE: Duration = Duration::from_secs(2);

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
    let outcome = drain_frames(&mut frames, |frame| {
        // Stamp publication at the moment of write, distinct from the receive
        // stamp taken at dequeue, so a consumer can separate host queueing
        // latency from the capture-to-receipt gap.
        let published_at_ns = now_ns(start);
        let connection = &connection;
        async move { write_one_frame(connection, source_id, &frame, published_at_ns).await }
    })
    .await;
    match outcome {
        DrainEnd::ChannelClosed => {}
        DrainEnd::WriteFailed(reason) => {
            // A failed uni-stream open/write means the connection is going
            // away; stop writing video to it. The connection task's own
            // teardown deregisters this client.
            debug!(
                client = client.as_u64(),
                source_id, reason, "video writer stopping"
            );
        }
    }
}

/// Why [`drain_frames`] returned.
#[derive(Debug, PartialEq, Eq)]
enum DrainEnd {
    /// The handoff channel closed (client deregistered or shutdown).
    ChannelClosed,
    /// A write failed outright; the connection is going away.
    WriteFailed(&'static str),
}

/// Drains the handoff channel, writing each frame under
/// [`FRAME_WRITE_DEADLINE`]. A write that exceeds the deadline is
/// abandoned (dropping the in-flight future drops its stream, which
/// resets it toward the peer) and the writer moves on to the next frame,
/// so a stalled consumer costs one frame rather than wedging the source
/// permanently. Only an outright write error ends the drain.
async fn drain_frames<W, Fut>(frames: &mut mpsc::Receiver<EncodedFrame>, mut write: W) -> DrainEnd
where
    W: FnMut(EncodedFrame) -> Fut,
    Fut: Future<Output = Result<(), &'static str>>,
{
    let mut stalls: u64 = 0;
    while let Some(frame) = frames.recv().await {
        match tokio::time::timeout(FRAME_WRITE_DEADLINE, write(frame)).await {
            Ok(Ok(())) => {}
            Ok(Err(reason)) => return DrainEnd::WriteFailed(reason),
            Err(_elapsed) => {
                stalls = stalls.wrapping_add(1);
                warn!(
                    total_stalls = stalls,
                    "video frame write exceeded its deadline; stream abandoned, continuing \
                     with the next frame"
                );
            }
        }
    }
    DrainEnd::ChannelClosed
}

/// Opens one host-initiated uni stream, writes the [`VIDEO_FRAME_V2`] tag then
/// the capture-identity header and codec-tagged, length-prefixed JPEG, and
/// finishes the stream (ADR-0005 media unit; ADR-0016 FourCC codec tag;
/// ADR-0020 capture identity).
async fn write_one_frame(
    connection: &Connection,
    source_id: u8,
    frame: &EncodedFrame,
    published_at_ns: u64,
) -> Result<(), &'static str> {
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
        return Ok(());
    };
    let mut stream = connection
        .open_uni()
        .await
        .map_err(|_| "uni stream request failed")?
        .await
        .map_err(|_| "uni stream failed to open")?;
    stream
        .write_all(&[VIDEO_FRAME_V2])
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
    use pilotage_adapter_api::{
        CalibrationId, CameraId, CaptureClockMapping, MeasurementClock, MeasurementStamp,
        SourceIncarnation, SourceIntegrity, SourceRole, VideoCaptureStamp,
    };

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

    fn encoded_frame() -> super::EncodedFrame {
        super::EncodedFrame {
            jpeg: std::sync::Arc::new(vec![0xFF, 0xD8, 0xFF, 0xD9]),
            capture: capture_stamp(),
            received_at_ns: 0,
        }
    }

    /// A write that never completes must cost exactly one frame: the
    /// deadline abandons it and the writer proceeds to the next frame
    /// (virtual time, no real waiting).
    #[tokio::test(start_paused = true)]
    async fn stalled_write_is_abandoned_and_the_next_frame_proceeds() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(2);
        tx.send(encoded_frame()).await.expect("first frame queues");
        tx.send(encoded_frame()).await.expect("second frame queues");
        drop(tx);

        let calls = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let seen = calls.clone();
        let end = super::drain_frames(&mut rx, move |_frame| {
            let call = seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async move {
                if call == 0 {
                    std::future::pending::<()>().await;
                }
                Ok(())
            }
        })
        .await;

        assert_eq!(end, super::DrainEnd::ChannelClosed);
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "the frame after the stalled one must still be written"
        );
    }

    /// An outright write error means the connection is going away: the
    /// drain ends rather than retrying into a dead connection.
    #[tokio::test(start_paused = true)]
    async fn write_error_ends_the_drain() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(2);
        tx.send(encoded_frame()).await.expect("frame queues");
        tx.send(encoded_frame()).await.expect("second frame queues");

        let end =
            super::drain_frames(&mut rx, |_frame| async { Err("uni stream request failed") }).await;

        assert_eq!(
            end,
            super::DrainEnd::WriteFailed("uni stream request failed")
        );
    }
}
