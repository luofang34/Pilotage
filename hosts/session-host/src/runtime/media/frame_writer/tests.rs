//! Frame-writer delivery tests: deadline stalls, peer-local
//! stop/refusal (one-frame loss, writer survives), and
//! connection-fatal loss (writer retires), all in virtual time.

#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use pilotage_adapter_api::{
    CalibrationId, CameraId, CaptureClockMapping, MeasurementClock, MeasurementStamp,
    SourceIncarnation, SourceIntegrity, SourceRole, VideoCaptureStamp,
};
use pilotage_session::ClientKey;
use tokio::sync::mpsc;
use tokio::time::Instant;

use wtransport::VarInt;
use wtransport::error::{StreamOpeningError, StreamWriteError};

use super::{
    EncodedFrame, FatalKind, FrameChannel, FrameStream, StreamError, classify_open, classify_write,
    drain_frames,
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
    /// `open()` never completes (the peer's stream allowance is full).
    Stall,
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

    async fn open(&self) -> Result<MockStream, StreamError> {
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
            Open::Refused => Err(classify_open(&StreamOpeningError::Refused)),
            Open::ConnFatal => Err(classify_open(&StreamOpeningError::NotConnected)),
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

    drain_frames(ClientKey::new(1), 0, &channel, &mut rx, Instant::now()).await;

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

    drain_frames(ClientKey::new(1), 0, &channel, &mut rx, Instant::now()).await;

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

    drain_frames(ClientKey::new(1), 0, &channel, &mut rx, Instant::now()).await;

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

    drain_frames(ClientKey::new(1), 0, &channel, &mut rx, Instant::now()).await;

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

    drain_frames(ClientKey::new(1), 0, &channel, &mut rx, Instant::now()).await;

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

// ---- direct classifier matrix: every pinned wtransport variant --------------
//
// The drain tests above route through `classify_write`/`classify_open`,
// but these pin the mapping itself so a reversed production classifier
// (peer-local vs connection-fatal swapped) fails outright.

#[test]
fn classify_write_maps_every_pinned_variant() {
    assert_eq!(
        classify_write(&StreamWriteError::Stopped(VarInt::from_u32(9)), "write"),
        StreamError::PeerStop {
            phase: "write",
            code: Some(9),
        },
    );
    assert_eq!(
        classify_write(&StreamWriteError::Closed, "finish"),
        StreamError::LocalClose { phase: "finish" },
    );
    assert_eq!(
        classify_write(&StreamWriteError::NotConnected, "write"),
        StreamError::ConnectionFatal {
            phase: "write",
            kind: FatalKind::NotConnected,
        },
    );
    assert_eq!(
        classify_write(&StreamWriteError::QuicProto, "write"),
        StreamError::ConnectionFatal {
            phase: "write",
            kind: FatalKind::QuicProto,
        },
    );
}

#[test]
fn classify_open_maps_every_pinned_variant() {
    assert_eq!(
        classify_open(&StreamOpeningError::Refused),
        StreamError::PeerStop {
            phase: "open",
            code: None,
        },
    );
    assert_eq!(
        classify_open(&StreamOpeningError::NotConnected),
        StreamError::ConnectionFatal {
            phase: "open",
            kind: FatalKind::NotConnected,
        },
    );
}
