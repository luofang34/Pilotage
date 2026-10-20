//! Reading of host-initiated unidirectional streams (ADR-0005): the dedicated
//! authority-events stream and the video streams.
//!
//! Every host-initiated uni stream leads with a 1-byte kind tag; each accepted
//! stream gets its own reader task so a slow video decode never blocks
//! accepting the next stream. Authority and multiplexed video streams stay
//! open and dispatch complete records as they arrive.

use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Instant;

use tokio::sync::mpsc;
use tracing::warn;
use wtransport::RecvStream;

use super::{ReceiverEvent, VideoCapture, drain_stream_frames, forward_event};
use crate::control_source::elapsed_to_timestamp;

/// Kind tag prefixing the host's authority-events uni stream (ADR-0005).
const AUTHORITY_EVENTS_TAG: u8 = 0x01;
/// Kind tag prefixing a per-frame video uni stream (ADR-0005).
const VIDEO_FRAME_TAG: u8 = 0x02;
/// Kind tag prefixing a per-frame capture-identity video stream.
const VIDEO_FRAME_V2_TAG: u8 = 0x03;
/// Kind tag prefixing a long-lived, length-delimited video stream.
const VIDEO_STREAM_V3_TAG: u8 = 0x04;
/// Byte count of the big-endian length prefix on each V3 record.
const VIDEO_RECORD_PREFIX_LEN: usize = 4;
/// Reject implausible declarations before buffering toward their length.
const MAX_VIDEO_RECORD_LEN: usize = 64 * 1024 * 1024;

/// Reads exactly one host-initiated uni stream to completion: the leading
/// kind-tag byte, then the tag-specific body, forwarding the decoded
/// [`ReceiverEvent`]s. Long-lived stream kinds are decoded incrementally.
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
        VIDEO_FRAME_TAG => {
            read_video_stream(stream, buf, chunk, start, events_tx, dropped_events).await;
        }
        VIDEO_FRAME_V2_TAG => {
            read_video_v2_stream(stream, buf, chunk, start, events_tx, dropped_events).await;
        }
        VIDEO_STREAM_V3_TAG => {
            read_video_record_stream(stream, buf, chunk, start, events_tx, dropped_events).await;
        }
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

/// Reads one capture-identity frame carried by the retired V2 stream kind.
async fn read_video_v2_stream(
    mut stream: RecvStream,
    mut buf: Vec<u8>,
    mut chunk: [u8; 64 * 1024],
    start: Instant,
    events_tx: mpsc::Sender<ReceiverEvent>,
    dropped_events: Arc<AtomicU64>,
) {
    loop {
        match stream.read(&mut chunk).await {
            Ok(Some(count)) => buf.extend_from_slice(&chunk[..count]),
            Ok(None) => {
                forward_video_v2_body(&buf, start, &events_tx, &dropped_events);
                return;
            }
            Err(source) => {
                warn!(%source, "video-frame V2 stream read failed");
                return;
            }
        }
    }
}

/// Reads repeated length-delimited V2 frame bodies from a live V3 stream.
async fn read_video_record_stream(
    mut stream: RecvStream,
    mut buf: Vec<u8>,
    mut chunk: [u8; 64 * 1024],
    start: Instant,
    events_tx: mpsc::Sender<ReceiverEvent>,
    dropped_events: Arc<AtomicU64>,
) {
    loop {
        if let Err(declared) = drain_video_records(&mut buf, start, &events_tx, &dropped_events) {
            warn!(
                declared,
                limit = MAX_VIDEO_RECORD_LEN,
                "video record exceeds probe buffer limit; dropping stream"
            );
            return;
        }
        match stream.read(&mut chunk).await {
            Ok(Some(count)) => buf.extend_from_slice(&chunk[..count]),
            Ok(None) => {
                if !buf.is_empty() {
                    warn!(
                        buffered = buf.len(),
                        "video stream closed with a partial record"
                    );
                }
                return;
            }
            Err(source) => {
                warn!(%source, "multiplexed video stream read failed");
                return;
            }
        }
    }
}

/// Forwards every complete V3 record in `buf`, retaining only a partial tail.
fn drain_video_records(
    buf: &mut Vec<u8>,
    start: Instant,
    events_tx: &mpsc::Sender<ReceiverEvent>,
    dropped_events: &AtomicU64,
) -> Result<(), usize> {
    loop {
        if buf.len() < VIDEO_RECORD_PREFIX_LEN {
            return Ok(());
        }
        let declared = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        if declared > MAX_VIDEO_RECORD_LEN {
            return Err(declared);
        }
        let end = VIDEO_RECORD_PREFIX_LEN + declared;
        if buf.len() < end {
            return Ok(());
        }
        forward_video_v2_body(
            &buf[VIDEO_RECORD_PREFIX_LEN..end],
            start,
            events_tx,
            dropped_events,
        );
        buf.drain(..end);
    }
}

/// Decodes and forwards one capture-identity video body.
fn forward_video_v2_body(
    body: &[u8],
    start: Instant,
    events_tx: &mpsc::Sender<ReceiverEvent>,
    dropped_events: &AtomicU64,
) {
    let frame = match pilotage_protocol::video_frame::decode_v2(body) {
        Ok(frame) => frame,
        Err(source) => {
            warn!(%source, "dropping malformed video frame body");
            return;
        }
    };
    if frame.codec != FOURCC_MJPEG {
        warn!(
            codec = %String::from_utf8_lossy(&frame.codec),
            "unknown video codec FourCC; skipping frame"
        );
        return;
    }
    forward_event(
        events_tx,
        dropped_events,
        ReceiverEvent::VideoFrame {
            source_id: frame.header.source_id,
            jpeg: frame.payload.to_vec(),
            received_at: elapsed_to_timestamp(start.elapsed()),
            capture: Some(VideoCapture {
                clock: frame.header.capture_clock,
                time_ns: frame.header.capture_time_ns,
            }),
        },
    );
}

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
    start: Instant,
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
                    received_at: elapsed_to_timestamp(start.elapsed()),
                    capture: None,
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
    use std::sync::atomic::{AtomicU64, Ordering};

    use pilotage_protocol::video_frame::{CaptureHeader, encode_v2};
    use tokio::sync::mpsc;

    use super::{
        FOURCC_MJPEG, VIDEO_RECORD_PREFIX_LEN, drain_video_records, try_take_video_payload,
    };
    use crate::receiver::ReceiverEvent;

    fn capture_header(source_id: u8, sequence: u32) -> CaptureHeader {
        CaptureHeader {
            source_id,
            source_epoch: 1,
            source_incarnation: [sequence as u8; 16],
            sequence,
            capture_time_ns: u64::from(sequence),
            capture_clock: 2,
            mapping_available: false,
            mapping_target_clock: 0,
            mapping_offset_ns: 0,
            clock_error_bound_ns: 0,
            receive_time_ns: u64::from(sequence),
            publication_time_ns: u64::from(sequence),
            camera_id: u32::from(source_id),
            calibration_id: 0,
        }
    }

    fn video_record(source_id: u8, sequence: u32, jpeg: &[u8]) -> Vec<u8> {
        let body = encode_v2(&capture_header(source_id, sequence), FOURCC_MJPEG, jpeg)
            .expect("small payload encodes");
        let mut record = Vec::with_capacity(VIDEO_RECORD_PREFIX_LEN + body.len());
        record.extend_from_slice(&(body.len() as u32).to_be_bytes());
        record.extend_from_slice(&body);
        record
    }

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

    #[test]
    fn multiplexed_records_dispatch_live_across_read_boundaries() {
        let first = video_record(0, 1, &[1, 2, 3]);
        let second = video_record(1, 2, &[4, 5]);
        let split = first.len() + 2;
        let wire = [first, second].concat();
        let (events_tx, mut events_rx) = mpsc::channel(4);
        let dropped = AtomicU64::new(0);
        let mut buf = wire[..split].to_vec();

        drain_video_records(&mut buf, std::time::Instant::now(), &events_tx, &dropped)
            .expect("partial second record waits");
        match events_rx
            .try_recv()
            .expect("first record dispatches before close")
        {
            ReceiverEvent::VideoFrame {
                source_id,
                jpeg,
                capture,
                ..
            } => {
                assert_eq!(source_id, 0);
                assert_eq!(jpeg, vec![1, 2, 3]);
                // The first record's header states capture time 1 on the
                // simulation clock (code 2).
                assert_eq!(
                    capture,
                    Some(crate::receiver::VideoCapture {
                        clock: 2,
                        time_ns: 1
                    })
                );
            }
            _ => panic!("expected the first video frame"),
        }
        assert!(events_rx.try_recv().is_err(), "partial record is retained");

        buf.extend_from_slice(&wire[split..]);
        drain_video_records(&mut buf, std::time::Instant::now(), &events_tx, &dropped)
            .expect("second record completes");
        match events_rx.try_recv().expect("second record dispatches") {
            ReceiverEvent::VideoFrame {
                source_id, jpeg, ..
            } => {
                assert_eq!(source_id, 1);
                assert_eq!(jpeg, vec![4, 5]);
            }
            _ => panic!("expected the second video frame"),
        }
        assert!(buf.is_empty(), "all complete records are consumed");
        assert_eq!(dropped.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn oversized_record_is_rejected_from_its_prefix() {
        let declared = super::MAX_VIDEO_RECORD_LEN + 1;
        let mut buf = (declared as u32).to_be_bytes().to_vec();
        let (events_tx, _events_rx) = mpsc::channel(1);
        let dropped = AtomicU64::new(0);

        assert_eq!(
            drain_video_records(&mut buf, std::time::Instant::now(), &events_tx, &dropped),
            Err(declared)
        );
        assert_eq!(buf.len(), VIDEO_RECORD_PREFIX_LEN);
    }
}
