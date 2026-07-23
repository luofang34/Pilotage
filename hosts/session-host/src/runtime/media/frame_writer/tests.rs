//! Frame-writer delivery tests: deadline stalls, peer-local
//! stop/refusal (one-frame loss, writer survives), and
//! connection-fatal loss (writer retires), all in virtual time.

#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use pilotage_adapter_api::{
    CalibrationId, CameraId, CaptureClockMapping, MeasurementClock, MeasurementStamp,
    SourceIncarnation, SourceIntegrity, SourceRole, VideoCaptureStamp,
};
use pilotage_session::ClientKey;
use tokio::sync::{Notify, mpsc};
use tokio::time::Instant;

use wtransport::VarInt;
use wtransport::error::{StreamOpeningError, StreamWriteError};

use super::{
    EncodedFrame, FrameChannel, FrameStream, StreamError, classify_open, classify_write,
    drain_frames,
};
use crate::runtime::media::budget::PressureSignals;

fn pressure() -> Arc<PressureSignals> {
    Arc::new(PressureSignals::default())
}

async fn drain_test_frames<C: FrameChannel>(
    channel: &C,
    frames: &mut mpsc::Receiver<EncodedFrame>,
) {
    drain_frames(
        ClientKey::new(1),
        0,
        channel,
        frames,
        pressure(),
        Instant::now(),
    )
    .await;
}

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
    credit_wait_cancelled: AtomicU32,
    header_cancelled: AtomicU32,
    finished: AtomicU32,
    reset: AtomicU32,
    priority: AtomicI32,
    header_started: Notify,
    header_release: Notify,
    reset_observed: Notify,
}

/// How a mock stream behaves once opened. The failure cases construct a
/// real `StreamWriteError` and route it through the production
/// `classify_write`, so the drain tests exercise the actual mapping — a
/// reversed classifier would flip these outcomes too.
#[derive(Clone, Copy)]
enum Write {
    /// `write_all` never completes (a wedged consumer).
    Stall,
    /// The peer stopped this stream alone (`StreamWriteError::Stopped`).
    PeerStopped(u32),
    /// The stream was already closed locally (`StreamWriteError::Closed`).
    Closed,
    /// The connection is gone (`StreamWriteError::NotConnected`).
    ConnFatal,
    /// Writes and finishes normally.
    Ok,
}

/// How a mock `open()` behaves; the failure cases route real
/// `StreamOpeningError`s through the production `classify_open`.
#[derive(Clone, Copy)]
enum Open {
    /// The allocation-free stream-credit wait never completes.
    CreditStall,
    /// Allocation completes, but the WebTransport header flush stalls.
    HeaderStall(Write),
    /// The peer refused this stream alone (`StreamOpeningError::Refused`).
    Refused,
    /// The connection is gone (`StreamOpeningError::NotConnected`).
    ConnFatal,
    /// `open()` yields a stream with the given write behavior.
    Ready(Write),
}

struct MockStream {
    write: Write,
    tally: Arc<Tally>,
}

impl FrameStream for MockStream {
    fn set_priority(&self, priority: i32) {
        self.tally.priority.store(priority, Ordering::SeqCst);
    }

    async fn write_all(&mut self, _buf: &[u8]) -> Result<(), StreamError> {
        match self.write {
            Write::Stall => {
                std::future::pending::<()>().await;
                Ok(())
            }
            Write::PeerStopped(code) => Err(classify_write(
                &StreamWriteError::Stopped(VarInt::from_u32(code)),
                "write",
            )),
            Write::Closed => Err(classify_write(&StreamWriteError::Closed, "write")),
            Write::ConnFatal => Err(classify_write(&StreamWriteError::NotConnected, "write")),
            Write::Ok => Ok(()),
        }
    }

    async fn finish(&mut self) -> Result<(), StreamError> {
        self.tally.finished.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn reset(&mut self) {
        self.tally.reset.fetch_add(1, Ordering::SeqCst);
        self.tally.reset_observed.notify_one();
    }
}

/// Hands out opens from a per-open script; opens beyond the script are
/// `Ready(Write::Ok)`.
struct MockChannel {
    script: Vec<Open>,
    tally: Arc<Tally>,
}

struct MockOpening {
    open: Open,
    tally: Arc<Tally>,
}

struct CreditWaitCancellationProbe {
    tally: Arc<Tally>,
}

impl Drop for CreditWaitCancellationProbe {
    fn drop(&mut self) {
        self.tally
            .credit_wait_cancelled
            .fetch_add(1, Ordering::SeqCst);
    }
}

struct HeaderCancellationProbe {
    tally: Arc<Tally>,
    completed: bool,
}

impl Drop for HeaderCancellationProbe {
    fn drop(&mut self) {
        if !self.completed {
            self.tally.header_cancelled.fetch_add(1, Ordering::SeqCst);
        }
    }
}

impl FrameChannel for MockChannel {
    type Stream = MockStream;
    type Opening = MockOpening;

    async fn request_open(&self) -> Result<MockOpening, StreamError> {
        let n = self.tally.opened.fetch_add(1, Ordering::SeqCst) as usize;
        let open = self
            .script
            .get(n)
            .copied()
            .unwrap_or(Open::Ready(Write::Ok));
        match open {
            Open::CreditStall => {
                let _cancellation_probe = CreditWaitCancellationProbe {
                    tally: self.tally.clone(),
                };
                std::future::pending::<Result<MockOpening, StreamError>>().await
            }
            _ => Ok(MockOpening {
                open,
                tally: self.tally.clone(),
            }),
        }
    }

    async fn finish_open(opening: MockOpening) -> Result<MockStream, StreamError> {
        let write = match opening.open {
            Open::CreditStall => std::future::pending::<Write>().await,
            Open::HeaderStall(write) => {
                let mut cancellation_probe = HeaderCancellationProbe {
                    tally: opening.tally.clone(),
                    completed: false,
                };
                opening.tally.header_started.notify_one();
                opening.tally.header_release.notified().await;
                cancellation_probe.completed = true;
                write
            }
            Open::Refused => return Err(classify_open(&StreamOpeningError::Refused)),
            Open::ConnFatal => return Err(classify_open(&StreamOpeningError::NotConnected)),
            Open::Ready(write) => write,
        };
        Ok(MockStream {
            write,
            tally: opening.tally,
        })
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

    drain_test_frames(&channel, &mut rx).await;

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
    assert_eq!(
        tally.priority.load(Ordering::SeqCst),
        super::VIDEO_STREAM_PRIORITY,
        "video streams are lower priority than control traffic"
    );
}

/// A stalled allocation-free credit wait costs one frame. It is safe to
/// cancel because no QUIC stream exists yet.
#[tokio::test(start_paused = true)]
async fn a_stalled_credit_wait_is_cancelled_and_the_next_frame_proceeds() {
    let mut rx = queue(2).await;
    let tally = Arc::new(Tally::default());
    let channel = MockChannel {
        script: vec![Open::CreditStall, Open::Ready(Write::Ok)],
        tally: tally.clone(),
    };

    drain_test_frames(&channel, &mut rx).await;

    assert_eq!(
        tally.opened.load(Ordering::SeqCst),
        2,
        "the next frame opens its own stream after the stalled open"
    );
    assert_eq!(
        tally.credit_wait_cancelled.load(Ordering::SeqCst),
        1,
        "the timed-out open future is cancelled and returns its stream allowance wait"
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

/// Once a QUIC stream is allocated, its header-flush future stays owned.
/// The reaper awaits it and sends RESET_STREAM instead of dropping a
/// headerless stream with FIN.
#[tokio::test(start_paused = true)]
async fn a_stalled_header_flush_is_reaped_with_reset_and_writing_continues() {
    let mut rx = queue(2).await;
    let tally = Arc::new(Tally::default());
    let channel = MockChannel {
        script: vec![Open::HeaderStall(Write::Ok), Open::Ready(Write::Ok)],
        tally: tally.clone(),
    };
    let reset_observed = tally.reset_observed.notified();
    let task = tokio::spawn(async move {
        drain_test_frames(&channel, &mut rx).await;
    });

    tally.header_started.notified().await;
    tokio::time::advance(super::FRAME_WRITE_DEADLINE).await;
    task.await.expect("frame writer joins");

    assert_eq!(
        tally.header_cancelled.load(Ordering::SeqCst),
        0,
        "the allocated stream's header future remains owned after its deadline"
    );
    assert_eq!(
        tally.finished.load(Ordering::SeqCst),
        1,
        "the next frame finishes while the first is reaped"
    );
    tally.header_release.notify_one();
    reset_observed.await;
    assert_eq!(
        tally.reset.load(Ordering::SeqCst),
        1,
        "the reaper resets the allocated stream exactly once"
    );
    assert_eq!(
        tally.header_cancelled.load(Ordering::SeqCst),
        0,
        "completing through the reaper never cancellation-drops the header future"
    );
}

/// A peer that STOPS one stream costs one frame, not the source: the
/// next frame opens its own stream and finishes, and the writer stays
/// alive (it drains every queued frame rather than returning early).
#[tokio::test(start_paused = true)]
async fn a_peer_stopped_write_loses_one_frame_and_the_writer_survives() {
    let mut rx = queue(2).await;
    let tally = Arc::new(Tally::default());
    let channel = MockChannel {
        script: vec![Open::Ready(Write::PeerStopped(7)), Open::Ready(Write::Ok)],
        tally: tally.clone(),
    };

    drain_test_frames(&channel, &mut rx).await;

    assert_eq!(
        tally.opened.load(Ordering::SeqCst),
        2,
        "the writer survives the peer stop and opens the next frame's stream"
    );
    assert_eq!(
        tally.finished.load(Ordering::SeqCst),
        1,
        "the frame after the stopped one finishes cleanly"
    );
}

/// A peer that REFUSES one stream open likewise costs one frame: the
/// next frame proceeds and the writer stays alive.
#[tokio::test(start_paused = true)]
async fn a_refused_open_loses_one_frame_and_the_next_proceeds() {
    let mut rx = queue(2).await;
    let tally = Arc::new(Tally::default());
    let channel = MockChannel {
        script: vec![Open::Refused, Open::Ready(Write::Ok)],
        tally: tally.clone(),
    };

    drain_test_frames(&channel, &mut rx).await;

    assert_eq!(
        tally.opened.load(Ordering::SeqCst),
        2,
        "a refused open costs one frame; the next frame opens its stream"
    );
    assert_eq!(
        tally.finished.load(Ordering::SeqCst),
        1,
        "the frame after the refused open finishes cleanly"
    );
}

/// Connection-level loss DOES retire the writer: after a connection-fatal
/// write, the frame after it is never opened.
#[tokio::test(start_paused = true)]
async fn a_connection_fatal_write_retires_the_writer() {
    let mut rx = queue(2).await;
    let tally = Arc::new(Tally::default());
    let channel = MockChannel {
        script: vec![Open::Ready(Write::ConnFatal), Open::Ready(Write::Ok)],
        tally: tally.clone(),
    };

    drain_test_frames(&channel, &mut rx).await;

    assert_eq!(
        tally.opened.load(Ordering::SeqCst),
        1,
        "the writer retires on connection loss; the next frame is never opened"
    );
}

/// A connection-fatal OPEN also retires the writer.
#[tokio::test(start_paused = true)]
async fn a_connection_fatal_open_retires_the_writer() {
    let mut rx = queue(2).await;
    let tally = Arc::new(Tally::default());
    let channel = MockChannel {
        script: vec![Open::ConnFatal, Open::Ready(Write::Ok)],
        tally: tally.clone(),
    };

    drain_test_frames(&channel, &mut rx).await;

    assert_eq!(
        tally.opened.load(Ordering::SeqCst),
        1,
        "the writer retires on a connection-fatal open; the next frame is never opened"
    );
}

/// A local `Closed` is recoverable frame-local loss: the next frame opens
/// and finishes, the writer survives, and no stream reset is issued (the
/// stream was already closed, not deadline-abandoned).
#[tokio::test(start_paused = true)]
async fn a_local_close_loses_one_frame_and_the_writer_survives() {
    let mut rx = queue(2).await;
    let tally = Arc::new(Tally::default());
    let channel = MockChannel {
        script: vec![Open::Ready(Write::Closed), Open::Ready(Write::Ok)],
        tally: tally.clone(),
    };

    drain_test_frames(&channel, &mut rx).await;

    assert_eq!(
        tally.opened.load(Ordering::SeqCst),
        2,
        "the writer survives a local close and opens the next frame's stream"
    );
    assert_eq!(
        tally.finished.load(Ordering::SeqCst),
        1,
        "the frame after the local close finishes cleanly"
    );
    assert_eq!(
        tally.reset.load(Ordering::SeqCst),
        0,
        "a local close does not reset a stream"
    );
}
