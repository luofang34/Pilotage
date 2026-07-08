//! Reading of host-initiated unidirectional streams (ADR-0005): the dedicated
//! authority-events stream and the per-frame video streams.
//!
//! Every host-initiated uni stream leads with a 1-byte kind tag; each accepted
//! stream is read to completion by a short-lived task so a slow video decode
//! never blocks accepting the next stream. The authority-events stream stays
//! open for the connection's lifetime (repeated length-delimited envelopes); a
//! video-frame stream carries exactly one
//! `[source_id][fourcc][u32 LE len][jpeg]` body (ADR-0016) and closes.

use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Instant;

use tokio::sync::mpsc;
use tracing::warn;
use wtransport::RecvStream;

use super::{ReceiverEvent, drain_stream_frames, forward_event};

/// Kind tag prefixing the host's authority-events uni stream (ADR-0005).
const AUTHORITY_EVENTS_TAG: u8 = 0x01;
/// Kind tag prefixing a per-frame video uni stream (ADR-0005).
const VIDEO_FRAME_TAG: u8 = 0x02;

/// Reads exactly one host-initiated uni stream to completion: the leading
/// kind-tag byte, then the tag-specific body, forwarding the decoded
/// [`ReceiverEvent`]s. The authority-events stream stays open for the
/// connection's lifetime and is read incrementally (repeated envelopes); a
/// video-frame stream carries exactly one framed JPEG and then closes.
pub(super) async fn read_one_uni_stream(
    mut stream: RecvStream,
    start: Instant,
    events_tx: mpsc::Sender<ReceiverEvent>,
    dropped_events: Arc<AtomicU64>,
) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    let Some(tag) = read_kind_tag(&mut stream, &mut buf, &mut chunk).await else {
        return;
    };
    match tag {
        AUTHORITY_EVENTS_TAG => {
            read_authority_stream(stream, buf, chunk, start, events_tx, dropped_events).await;
        }
        VIDEO_FRAME_TAG => read_video_stream(stream, buf, chunk, events_tx, dropped_events).await,
        other => warn!(
            tag = other,
            "unrecognized uni-stream kind tag; dropping stream"
        ),
    }
}

/// Reads bytes until at least one is available, then returns the leading
/// kind-tag byte, leaving any further bytes already read in `buf`.
async fn read_kind_tag(stream: &mut RecvStream, buf: &mut Vec<u8>, chunk: &mut [u8]) -> Option<u8> {
    while buf.is_empty() {
        match stream.read(chunk).await {
            Ok(Some(count)) => buf.extend_from_slice(&chunk[..count]),
            Ok(None) => {
                warn!("uni stream closed before its kind tag arrived");
                return None;
            }
            Err(source) => {
                warn!(%source, "uni stream read failed before its kind tag arrived");
                return None;
            }
        }
    }
    Some(buf.remove(0))
}

/// Reads the dedicated authority-events stream for the remainder of its
/// (connection-lifetime) duration, decoding and forwarding every
/// length-delimited envelope as it arrives.
async fn read_authority_stream(
    mut stream: RecvStream,
    mut buf: Vec<u8>,
    mut chunk: [u8; 64 * 1024],
    start: Instant,
    events_tx: mpsc::Sender<ReceiverEvent>,
    dropped_events: Arc<AtomicU64>,
) {
    drain_stream_frames(&mut buf, start, &events_tx, &dropped_events);
    loop {
        match stream.read(&mut chunk).await {
            Ok(Some(count)) => {
                buf.extend_from_slice(&chunk[..count]);
                drain_stream_frames(&mut buf, start, &events_tx, &dropped_events);
            }
            Ok(None) => {
                warn!("authority-events stream closed");
                return;
            }
            Err(source) => {
                warn!(%source, "authority-events stream read failed");
                return;
            }
        }
    }
}

/// FourCC of the only video codec this viewer decodes: Motion JPEG (ADR-0016).
const FOURCC_MJPEG: [u8; 4] = *b"MJPG";

/// Reads one video-frame stream to completion
/// (`[source_id][fourcc][u32 LE len][payload]`, ADR-0016), then forwards the
/// JPEG bytes (tagged with their `source_id`) as a single
/// [`ReceiverEvent::VideoFrame`]. A frame whose FourCC is not [`FOURCC_MJPEG`]
/// is skipped with a warning, never a hard failure, so a host streaming a
/// codec this viewer lacks degrades gracefully.
async fn read_video_stream(
    mut stream: RecvStream,
    mut buf: Vec<u8>,
    mut chunk: [u8; 64 * 1024],
    events_tx: mpsc::Sender<ReceiverEvent>,
    dropped_events: Arc<AtomicU64>,
) {
    loop {
        if let Some((source_id, fourcc, payload)) = try_take_video_payload(&mut buf) {
            if fourcc != FOURCC_MJPEG {
                warn!(
                    codec = %String::from_utf8_lossy(&fourcc),
                    "unknown video codec FourCC; skipping frame"
                );
                return;
            }
            forward_event(
                &events_tx,
                &dropped_events,
                ReceiverEvent::VideoFrame {
                    source_id,
                    jpeg: payload,
                },
            );
            return;
        }
        match stream.read(&mut chunk).await {
            Ok(Some(count)) => buf.extend_from_slice(&chunk[..count]),
            Ok(None) => {
                warn!("video-frame stream closed before a complete frame arrived");
                return;
            }
            Err(source) => {
                warn!(%source, "video-frame stream read failed");
                return;
            }
        }
    }
}

/// Parses a `[source_id: 1 byte][fourcc: 4 bytes][u32 LE len][payload]` body
/// (ADR-0016) once `buf` holds the declared length in full, returning the
/// `(source_id, fourcc, payload)` triple and leaving `buf` empty. Returns
/// `None` if fewer than the 9-byte source-id+fourcc+length header, or fewer
/// than the declared payload length, have arrived yet.
fn try_take_video_payload(buf: &mut Vec<u8>) -> Option<(u8, [u8; 4], Vec<u8>)> {
    const HEADER: usize = 9;
    if buf.len() < HEADER {
        return None;
    }
    let source_id = buf[0];
    let fourcc = [buf[1], buf[2], buf[3], buf[4]];
    let declared = u32::from_le_bytes([buf[5], buf[6], buf[7], buf[8]]);
    let declared = usize::try_from(declared).ok()?;
    if buf.len() < HEADER + declared {
        return None;
    }
    let payload = buf[HEADER..HEADER + declared].to_vec();
    buf.drain(..HEADER + declared);
    Some((source_id, fourcc, payload))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{FOURCC_MJPEG, try_take_video_payload};

    #[test]
    fn video_payload_waits_for_full_declared_length() {
        // [src=1][MJPG][len=3][1,2] — one payload byte still missing.
        let mut buf = vec![1, b'M', b'J', b'P', b'G', 3, 0, 0, 0, 1, 2];
        assert_eq!(
            try_take_video_payload(&mut buf),
            None,
            "only 2 of 3 payload bytes present"
        );
        buf.push(3);
        let (source_id, codec, jpeg) =
            try_take_video_payload(&mut buf).expect("full frame present");
        assert_eq!(source_id, 1, "reads the chase source id");
        assert_eq!(codec, FOURCC_MJPEG);
        assert_eq!(jpeg, vec![1, 2, 3]);
        assert!(buf.is_empty(), "consumed bytes are drained");
    }

    #[test]
    fn video_payload_leaves_trailing_bytes_for_the_next_frame() {
        // [src=0][MJPG][len=1][0xAB] then the start of a second frame.
        let mut buf = vec![0, b'M', b'J', b'P', b'G', 1, 0, 0, 0, 0xAB, b'M', b'J'];
        let (source_id, _, first) = try_take_video_payload(&mut buf).expect("first frame present");
        assert_eq!(source_id, 0, "reads the FPV source id");
        assert_eq!(first, vec![0xAB]);
        assert_eq!(buf, vec![b'M', b'J']);
    }
}
