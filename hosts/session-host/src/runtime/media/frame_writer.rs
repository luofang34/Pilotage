//! Per-(client, source) video frame writer: one host-initiated uni
//! stream per frame, written under a deadline so a stalled consumer
//! costs one frame rather than wedging its source permanently.

use std::future::Future;
use std::time::Duration;

use pilotage_session::ClientKey;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tracing::{debug, error, warn};
use wtransport::error::{StreamOpeningError, StreamWriteError};
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

/// Why an open, write, or finish failed, classified so a peer's decision
/// to abandon ONE stream does not retire the whole source writer.
enum StreamError {
    /// The peer stopped or refused this stream alone (`Stopped`,
    /// `Refused`), or it was already closed locally. The connection and
    /// every other source are unaffected — this is one-frame loss.
    PeerLocal {
        /// The phase that surfaced it (`open`, `write`, `finish`).
        phase: &'static str,
        /// The peer's application error code, when it carried one.
        code: Option<u64>,
    },
    /// Connection-level loss or a protocol failure (`NotConnected`,
    /// `QuicProto`, or an open-request `ConnectionError`): the writer must
    /// retire — no further frame can be delivered on this connection.
    ConnectionFatal(&'static str),
}

/// Classifies a wtransport write/finish error: only `NotConnected` and
/// `QuicProto` are connection-fatal; `Stopped`/`Closed` are peer-local.
fn classify_write(error: &StreamWriteError, phase: &'static str) -> StreamError {
    match error {
        StreamWriteError::Stopped(code) => StreamError::PeerLocal {
            phase,
            code: Some(code.into_inner()),
        },
        StreamWriteError::Closed => StreamError::PeerLocal { phase, code: None },
        StreamWriteError::NotConnected => StreamError::ConnectionFatal("not connected"),
        StreamWriteError::QuicProto => StreamError::ConnectionFatal("QUIC protocol error"),
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
    type Stream: FrameStream;
    /// Opens one host-initiated uni stream.
    fn open(&self) -> impl Future<Output = Result<Self::Stream, StreamError>> + Send;
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

    async fn open(&self) -> Result<SendStream, StreamError> {
        // The open-request error is connection-level; the opening error can
        // be a peer refusal of this stream alone.
        let opening = self
            .open_uni()
            .await
            .map_err(|_| StreamError::ConnectionFatal("uni stream request failed"))?;
        opening.await.map_err(|e| match e {
            StreamOpeningError::Refused => StreamError::PeerLocal {
                phase: "open",
                code: None,
            },
            StreamOpeningError::NotConnected => StreamError::ConnectionFatal("not connected"),
        })
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
    /// The peer stopped or refused this stream alone; the connection and
    /// other sources are healthy — one-frame loss, keep writing.
    PeerLocal {
        /// The phase (`open`, `write`, `finish`) that surfaced it.
        phase: &'static str,
        /// The peer's application error code, when it carried one.
        code: Option<u64>,
    },
    /// Connection-level loss or protocol failure; retire the writer.
    ConnectionFatal(&'static str),
}

/// Drains the handoff channel, delivering one frame per stream under a
/// single absolute per-frame deadline that covers BOTH opening the stream
/// and writing it. A frame lost to a deadline or a peer-local stop/refusal
/// costs one frame and the writer proceeds; only connection-level loss
/// retires the writer.
async fn drain_frames<C: FrameChannel>(
    client: ClientKey,
    source_id: u8,
    channel: &C,
    frames: &mut mpsc::Receiver<EncodedFrame>,
    start: Instant,
) {
    let mut stalls: u64 = 0;
    let mut peer_drops: u64 = 0;
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
            FrameOutcome::PeerLocal { phase, code } => {
                peer_drops = peer_drops.wrapping_add(1);
                warn!(
                    client = client.as_u64(),
                    source_id,
                    phase,
                    peer_code = code,
                    total_peer_drops = peer_drops,
                    "peer stopped or refused this video stream; the connection is healthy, \
                     continuing with the next frame"
                );
            }
            FrameOutcome::ConnectionFatal(reason) => {
                // Connection-level loss; stop writing video to it. The
                // connection task's own teardown deregisters this client.
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
/// reset can fire rather than a dropped-future FIN. Typed open/write
/// failures pass their peer-local vs connection-fatal classification
/// through unchanged.
async fn deliver_frame<C: FrameChannel>(
    channel: &C,
    deadline: Instant,
    tag: u8,
    body: &[u8],
) -> FrameOutcome {
    let mut stream = match tokio::time::timeout_at(deadline, channel.open()).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => return error.into_outcome(),
        Err(_elapsed) => return FrameOutcome::Stalled { reset: false },
    };
    match tokio::time::timeout_at(deadline, write_body(&mut stream, tag, body)).await {
        Ok(Ok(())) => FrameOutcome::Sent,
        Ok(Err(error)) => error.into_outcome(),
        Err(_elapsed) => {
            stream.reset();
            FrameOutcome::Stalled { reset: true }
        }
    }
}

impl StreamError {
    /// Maps a stream error to its frame outcome, preserving the peer-local
    /// vs connection-fatal classification.
    fn into_outcome(self) -> FrameOutcome {
        match self {
            StreamError::PeerLocal { phase, code } => FrameOutcome::PeerLocal { phase, code },
            StreamError::ConnectionFatal(reason) => FrameOutcome::ConnectionFatal(reason),
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
mod tests;
