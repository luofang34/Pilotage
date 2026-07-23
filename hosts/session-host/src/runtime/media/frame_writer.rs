//! Per-(client, source) video frame writer: one host-initiated uni
//! stream per frame, written under a deadline so a stalled consumer
//! costs one frame rather than wedging its source permanently.

use std::future::Future;
use std::time::Duration;

use pilotage_session::ClientKey;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tracing::{error, warn};
use wtransport::error::{ConnectionError, StreamOpeningError, StreamWriteError};
use wtransport::stream::OpeningUniStream;
use wtransport::{Connection, SendStream, VarInt};

use super::{EncodedFrame, now_ns};
use crate::runtime::stream_tag::{FOURCC_MJPEG, VIDEO_FRAME_V2, frame_video_payload_v2};

use reaper::OpenReapers;

mod reaper;

/// Longest a single frame's uni-stream write may take before the stream
/// is reset and the writer moves on. A client that stops consuming one
/// stream (per-stream flow control fills) would otherwise park the write
/// forever: the capacity-1 handoff then never frees, every later frame is
/// dropped-to-latest, and that source is dead for that client until
/// reconnect — a wedged stream must cost one frame, not the source.
/// Generous against transient congestion (a healthy frame completes in
/// milliseconds on the deployment link).
const FRAME_WRITE_DEADLINE: Duration = Duration::from_secs(2);

/// A peer that withholds uni-stream credit for this long is starving the
/// connection, not merely delaying one frame. This stage allocates no stream,
/// so cancellation is safe and later frames may retry loudly.
const STREAM_CREDIT_STARVATION_BOUND: Duration = Duration::from_secs(30);

/// Application error code carried on the RESET_STREAM of a
/// deadline-exceeded frame. Informational to the peer, which discards the
/// partial frame regardless of the code.
const STALL_RESET_CODE: u32 = 1;

/// A connection-fatal error's kind, preserved rather than erased so the
/// log names — and the error chain carries — what actually failed. The
/// open-request kind keeps the real [`ConnectionError`] as its `#[source]`
/// (timeout, application-close reason/code, H3, QUIC details all survive).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
enum FatalKind {
    /// The connection has been dropped.
    #[error("not connected")]
    NotConnected,
    /// A QUIC protocol error.
    #[error("QUIC protocol error")]
    QuicProto,
    /// The uni-stream open request itself failed (connection-level); the
    /// concrete cause is preserved.
    #[error("uni stream request failed: {0}")]
    OpenRequest(#[source] ConnectionError),
}

/// Why an open, write, or finish failed, classified so neither a peer's
/// decision to abandon ONE stream nor a local close retires the writer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
enum StreamError {
    /// The peer stopped or refused this stream alone (`Stopped`,
    /// `Refused`): a peer-attributed one-frame loss; the connection and
    /// every other source are unaffected.
    #[error("peer stopped or refused the stream at {phase}")]
    PeerStop {
        /// The phase that surfaced it (`open`, `write`, `finish`).
        phase: &'static str,
        /// The peer's application error code, when it carried one.
        code: Option<u64>,
    },
    /// The stream was already closed locally (`Closed`): a frame-local
    /// anomaly, recoverable like a peer stop but NOT a peer stop/refusal.
    #[error("stream already closed locally at {phase}")]
    LocalClose {
        /// The phase that surfaced it.
        phase: &'static str,
    },
    /// Connection-level loss or a protocol failure: the writer must
    /// retire — no further frame can be delivered on this connection.
    #[error("connection-fatal at {phase}: {kind}")]
    ConnectionFatal {
        /// The phase that surfaced it.
        phase: &'static str,
        /// The preserved underlying kind (its own `#[source]` chain).
        #[source]
        kind: FatalKind,
    },
}

/// Classifies a wtransport write/finish error: `Stopped` is a peer stop,
/// `Closed` is a local close, `NotConnected`/`QuicProto` are
/// connection-fatal. Pinned to wtransport 0.7.1's four variants.
fn classify_write(error: &StreamWriteError, phase: &'static str) -> StreamError {
    match error {
        StreamWriteError::Stopped(code) => StreamError::PeerStop {
            phase,
            code: Some(code.into_inner()),
        },
        StreamWriteError::Closed => StreamError::LocalClose { phase },
        StreamWriteError::NotConnected => StreamError::ConnectionFatal {
            phase,
            kind: FatalKind::NotConnected,
        },
        StreamWriteError::QuicProto => StreamError::ConnectionFatal {
            phase,
            kind: FatalKind::QuicProto,
        },
    }
}

/// Classifies a wtransport stream-opening error: `Refused` is a peer
/// refusal of this stream alone, `NotConnected` is connection-fatal.
/// Pinned to wtransport 0.7.1's two variants.
fn classify_open(error: &StreamOpeningError) -> StreamError {
    match error {
        StreamOpeningError::Refused => StreamError::PeerStop {
            phase: "open",
            code: None,
        },
        StreamOpeningError::NotConnected => StreamError::ConnectionFatal {
            phase: "open",
            kind: FatalKind::NotConnected,
        },
    }
}

/// Classifies an open-request [`ConnectionError`] (the first `open_uni`
/// await): always connection-fatal, but the concrete cause is retained
/// rather than discarded to a static string.
fn classify_open_request(error: ConnectionError) -> StreamError {
    StreamError::ConnectionFatal {
        phase: "open",
        kind: FatalKind::OpenRequest(error),
    }
}

/// One per-frame outbound stream. `write_all`/`finish` are the clean send
/// path; `reset` is the explicit RESET_STREAM a deadline-exceeded frame
/// needs. Dropping a wtransport/Quinn `SendStream` attempts a graceful
/// FIN, not a reset — a stalled peer never drains it, so truncated
/// streams would linger and eventually exhaust its stream allowance.
/// Resetting the retained stream frees its slot immediately. Send-bounded
/// because the writer runs as a spawned task on a multi-threaded runtime.
trait FrameStream {
    /// Writes the whole buffer, awaiting flow-control credit.
    fn write_all(&mut self, buf: &[u8]) -> impl Future<Output = Result<(), StreamError>> + Send;
    /// Finishes the stream cleanly (graceful FIN).
    fn finish(&mut self) -> impl Future<Output = Result<(), StreamError>> + Send;
    /// Resets the stream (RESET_STREAM); a no-op on an already-closed one.
    fn reset(&mut self);
}

/// Opens per-frame outbound streams on a connection.
trait FrameChannel {
    /// The stream this channel opens.
    type Stream: FrameStream + Send + 'static;
    /// The allocated stream while its WebTransport header is being flushed.
    type Opening: Send + 'static;
    /// Waits for stream credit, returning only after allocation.
    fn request_open(&self) -> impl Future<Output = Result<Self::Opening, StreamError>> + Send;
    /// Flushes the WebTransport stream header for an allocated stream.
    fn finish_open(
        opening: Self::Opening,
    ) -> impl Future<Output = Result<Self::Stream, StreamError>> + Send + 'static;
}

impl FrameStream for SendStream {
    async fn write_all(&mut self, buf: &[u8]) -> Result<(), StreamError> {
        SendStream::write_all(self, buf)
            .await
            .map_err(|e| classify_write(&e, "write"))
    }

    async fn finish(&mut self) -> Result<(), StreamError> {
        SendStream::finish(self)
            .await
            .map_err(|e| classify_write(&e, "finish"))
    }

    fn reset(&mut self) {
        // Best-effort: a stream already finished or reset returns
        // ClosedStream, which is nothing to act on here.
        SendStream::reset(self, VarInt::from_u32(STALL_RESET_CODE)).ok();
    }
}

impl FrameChannel for Connection {
    type Stream = SendStream;
    type Opening = OpeningUniStream;

    async fn request_open(&self) -> Result<OpeningUniStream, StreamError> {
        self.open_uni().await.map_err(classify_open_request)
    }

    async fn finish_open(opening: OpeningUniStream) -> Result<SendStream, StreamError> {
        opening.await.map_err(|e| classify_open(&e))
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeadlinePhase {
    CreditWait,
    HeaderFlush,
    Write,
}

impl DeadlinePhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::CreditWait => "open_credit",
            Self::HeaderFlush => "open_header",
            Self::Write => "write",
        }
    }
}

#[derive(Debug)]
enum FrameOutcome {
    /// Written and finished cleanly.
    Sent,
    /// The frame exceeded its deadline. `reset` is true when the stream
    /// was immediately reset. `reaper_owned` is true when an allocated
    /// stream is still opening under detached ownership.
    Stalled {
        phase: DeadlinePhase,
        reset: bool,
        reaper_owned: bool,
    },
    /// The peer stopped or refused this stream alone; the connection and
    /// other sources are healthy — one-frame loss, keep writing.
    PeerStop {
        /// The phase (`open`, `write`, `finish`) that surfaced it.
        phase: &'static str,
        /// The peer's application error code, when it carried one.
        code: Option<u64>,
    },
    /// The stream was already closed locally — a frame-local anomaly,
    /// recoverable, but not a peer stop/refusal.
    LocalClose {
        /// The phase that surfaced it.
        phase: &'static str,
    },
    /// Connection-level loss or protocol failure; retire the writer.
    ConnectionFatal {
        /// The phase that surfaced it.
        phase: &'static str,
        /// The preserved underlying kind.
        kind: FatalKind,
    },
}

/// The running per-writer loss counters, kept distinguishable so logs and
/// metrics separate deadline stalls, peer-local stop/refusal, and
/// local closes from the one connection-fatal event that ends the writer.
#[derive(Default)]
struct LossCounters {
    stalls: u64,
    peer_drops: u64,
    local_closes: u64,
}

/// Drains the handoff channel, delivering one frame per stream. Stream-credit
/// waits have an allocation-free starvation bound; header flush and writing
/// share a per-frame deadline. Frame-local loss keeps the writer alive, while
/// connection-level loss retires it.
async fn drain_frames<C: FrameChannel>(
    client: ClientKey,
    source_id: u8,
    channel: &C,
    frames: &mut mpsc::Receiver<EncodedFrame>,
    start: Instant,
) {
    let mut counters = LossCounters::default();
    let mut reapers = OpenReapers::new(client, source_id);
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
        let outcome = deliver_frame(
            channel,
            &mut reapers,
            STREAM_CREDIT_STARVATION_BOUND,
            FRAME_WRITE_DEADLINE,
            VIDEO_FRAME_V2,
            &body,
        )
        .await;
        if !record_outcome(client, source_id, outcome, &mut counters) {
            return;
        }
    }
}

/// Folds one frame's outcome into the counters and logs it, returning
/// `false` when the outcome is connection-fatal and the writer must retire.
fn record_outcome(
    client: ClientKey,
    source_id: u8,
    outcome: FrameOutcome,
    counters: &mut LossCounters,
) -> bool {
    match outcome {
        FrameOutcome::Sent => {}
        FrameOutcome::Stalled {
            phase,
            reset,
            reaper_owned,
        } => {
            counters.stalls = counters.stalls.wrapping_add(1);
            warn!(
                client = client.as_u64(),
                source_id,
                phase = phase.as_str(),
                total_stalls = counters.stalls,
                stream_reset = reset,
                reaper_owned,
                "video frame exceeded its deadline; continuing with the next frame"
            );
        }
        FrameOutcome::PeerStop { phase, code } => {
            counters.peer_drops = counters.peer_drops.wrapping_add(1);
            warn!(
                client = client.as_u64(),
                source_id,
                phase,
                peer_code = code,
                total_peer_drops = counters.peer_drops,
                "peer stopped or refused this video stream; the connection is healthy, \
                 continuing with the next frame"
            );
        }
        FrameOutcome::LocalClose { phase } => {
            counters.local_closes = counters.local_closes.wrapping_add(1);
            warn!(
                client = client.as_u64(),
                source_id,
                phase,
                total_local_closes = counters.local_closes,
                "video stream was already closed locally; frame-local loss, continuing \
                 with the next frame"
            );
        }
        FrameOutcome::ConnectionFatal { phase, kind } => {
            // Connection-level loss retires the writer; a distinguishable
            // fatal record, not a debug line. `%source` logs the preserved
            // cause (timeout, application-close reason/code, H3, QUIC), not a
            // static string. The connection task's own teardown deregisters
            // this client.
            warn!(
                client = client.as_u64(),
                source_id,
                phase,
                source = %kind,
                total_connection_failures = 1_u64,
                total_stalls = counters.stalls,
                total_peer_drops = counters.peer_drops,
                total_local_closes = counters.local_closes,
                "video writer stopping: connection-level failure"
            );
            return false;
        }
    }
    true
}

/// Opens a stream and writes the tag then the framed body. Stream-credit wait
/// has its own allocation-free bound. Once allocated, the header flush and
/// body write share the frame budget; an expired header flush transfers its
/// still-live future to the reaper, while an expired body write resets the
/// stream immediately.
async fn deliver_frame<C: FrameChannel>(
    channel: &C,
    reapers: &mut OpenReapers,
    credit_bound: Duration,
    frame_budget: Duration,
    tag: u8,
    body: &[u8],
) -> FrameOutcome {
    let Some(reaper_permit) = reapers.reserve().await else {
        error!("video open reaper semaphore closed; retiring writer");
        return FrameOutcome::ConnectionFatal {
            phase: "open_header",
            kind: FatalKind::NotConnected,
        };
    };
    let opening = match tokio::time::timeout(credit_bound, channel.request_open()).await {
        Ok(Ok(opening)) => opening,
        Ok(Err(error)) => return error.into_outcome(),
        Err(_elapsed) => {
            return FrameOutcome::Stalled {
                phase: DeadlinePhase::CreditWait,
                reset: false,
                reaper_owned: false,
            };
        }
    };
    let deadline = Instant::now() + frame_budget;
    let mut completion = Box::pin(C::finish_open(opening));
    let mut stream = match tokio::time::timeout_at(deadline, &mut completion).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => return error.into_outcome(),
        Err(_elapsed) => {
            reapers.own(completion, reaper_permit);
            return FrameOutcome::Stalled {
                phase: DeadlinePhase::HeaderFlush,
                reset: false,
                reaper_owned: true,
            };
        }
    };
    drop(reaper_permit);
    match tokio::time::timeout_at(deadline, write_body(&mut stream, tag, body)).await {
        Ok(Ok(())) => FrameOutcome::Sent,
        Ok(Err(error)) => error.into_outcome(),
        Err(_elapsed) => {
            stream.reset();
            FrameOutcome::Stalled {
                phase: DeadlinePhase::Write,
                reset: true,
                reaper_owned: false,
            }
        }
    }
}

impl StreamError {
    /// Maps a stream error to its frame outcome, preserving the peer-stop,
    /// local-close, and connection-fatal classification.
    fn into_outcome(self) -> FrameOutcome {
        match self {
            StreamError::PeerStop { phase, code } => FrameOutcome::PeerStop { phase, code },
            StreamError::LocalClose { phase } => FrameOutcome::LocalClose { phase },
            StreamError::ConnectionFatal { phase, kind } => {
                FrameOutcome::ConnectionFatal { phase, kind }
            }
        }
    }
}

/// Writes the one-byte tag, then the body, then finishes the stream.
async fn write_body<S: FrameStream>(
    stream: &mut S,
    tag: u8,
    body: &[u8],
) -> Result<(), StreamError> {
    stream.write_all(&[tag]).await?;
    stream.write_all(body).await?;
    stream.finish().await
}

#[cfg(test)]
mod classification_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod transport_tests;
