//! Kind tags that disambiguate the host's several host-initiated
//! unidirectional streams (ADR-0005).
//!
//! Both authority events and video frames travel on host-initiated uni
//! streams, so a reader accepting a fresh uni stream cannot tell them apart
//! from the QUIC stream type alone. Every host-initiated uni stream therefore
//! begins with a single kind-tag byte before its payload; the reader consumes
//! that byte first and routes on it.

use pilotage_adapter_api::{CaptureClockMapping, MeasurementClock, VideoCaptureStamp};
use pilotage_protocol::{CaptureHeader, encode_video_frame_v2};

/// Tag prefixing the dedicated authority-events stream: the reliable, ordered
/// stream of lease/handover/override events (ADR-0005) carries
/// length-delimited envelopes after this byte.
pub const AUTHORITY_EVENTS: u8 = 0x01;

/// Tag prefixing a per-frame video stream: a
/// `[source_id: u8][fourcc: 4 bytes][u32 LE len][payload]` media unit follows
/// this byte (ADR-0005 media = one uni stream per frame; ADR-0016 codec tag).
/// `source_id` names the video source (0 = onboard FPV, 1 = chase) so a client
/// with several sources routes each frame to the right one.
///
/// This framing carries no capture identity; [`VIDEO_FRAME_V2`] supersedes it
/// as the emitted format. The tag and its builder are kept only to pin the
/// stable meaning of `0x02` in the compat tests.
#[cfg(test)]
pub const VIDEO_FRAME: u8 = 0x02;

/// Tag prefixing a per-frame video stream that leads with a capture-identity
/// header before the codec-tagged, length-prefixed payload (ADR-0020).
///
/// A reader that does not recognize this tag skips the stream, exactly as it
/// would an unknown FourCC, so a host emitting v2 frames degrades gracefully
/// against an older client. The body layout is built by
/// [`frame_video_payload_v2`].
pub const VIDEO_FRAME_V2: u8 = 0x03;

/// Wire code for a [`MeasurementClock`], matching the `pilotage.v1`
/// `MeasurementClock` enum the browser already decodes: 1 vehicle-boot, 2
/// simulation, 3 host-monotonic. `0` (unspecified) is reserved for an
/// absent target clock.
fn measurement_clock_code(clock: MeasurementClock) -> u8 {
    match clock {
        MeasurementClock::VehicleBoot => 1,
        MeasurementClock::Simulation => 2,
        MeasurementClock::HostMonotonic => 3,
    }
}

/// Serializes one encoded frame as the on-wire body that follows the
/// [`VIDEO_FRAME_V2`] tag: the fixed capture-identity header (source identity,
/// attachment epoch and incarnation, wrapping sequence, sim capture time and
/// clock, the clock mapping to the flight-state clock, host receive and
/// publication times, and camera/calibration identities), then the codec
/// FourCC, a little-endian `u32` length prefix, and the encoded bytes.
///
/// `source_id` is the routing byte (0 = onboard FPV, 1 = chase) and is the
/// same identity carried in `capture.stamp.source_id`. `receive_time_ns` and
/// `publication_time_ns` are host monotonic stamps, kept distinct from the
/// capture time so a consumer never conflates host receipt with acquisition.
///
/// Returns the framed bytes, or `None` if `payload` is larger than `u32::MAX`.
#[must_use]
pub fn frame_video_payload_v2(
    source_id: u8,
    capture: &VideoCaptureStamp,
    receive_time_ns: u64,
    publication_time_ns: u64,
    codec: [u8; 4],
    payload: &[u8],
) -> Option<Vec<u8>> {
    let (mapping_available, mapping_target_clock, mapping_offset_ns, clock_error_bound_ns) =
        match capture.mapping {
            CaptureClockMapping::Unavailable => (false, 0_u8, 0_i64, 0_u64),
            CaptureClockMapping::Bounded {
                target,
                offset_ns,
                error_bound_ns,
            } => (
                true,
                measurement_clock_code(target),
                offset_ns,
                error_bound_ns,
            ),
        };
    let stamp = &capture.stamp;
    let header = CaptureHeader {
        source_id,
        source_epoch: stamp.source_epoch,
        source_incarnation: stamp.source_incarnation.into_bytes(),
        sequence: stamp.sequence,
        capture_time_ns: stamp.acquired_at_ns,
        capture_clock: measurement_clock_code(stamp.clock),
        mapping_available,
        mapping_target_clock,
        mapping_offset_ns,
        clock_error_bound_ns,
        receive_time_ns,
        publication_time_ns,
        camera_id: capture.camera_id.0,
        calibration_id: capture.calibration_id.0,
    };
    encode_video_frame_v2(&header, codec, payload)
}

/// FourCC codec tag for Motion JPEG frames (ADR-0016).
///
/// The per-frame video body is `[fourcc][u32 LE len][payload]`; the client
/// routes on the FourCC and MUST treat an unknown one as a skipped frame, so
/// a host streaming a codec an older client lacks degrades gracefully rather
/// than hard-failing. Motion JPEG has a real FourCC (`MJPG`); the reserved
/// video codecs (`avc1`, `hvc1`, `av01`, `vp09`) are ISO-BMFF sample-entry
/// codes from MP4RA, so the value space is already arbitrated.
pub const FOURCC_MJPEG: [u8; 4] = *b"MJPG";

/// Serializes one encoded frame as the on-wire video-stream body that follows
/// the [`VIDEO_FRAME`] tag: a 1-byte `source_id` (0 = onboard FPV, 1 = chase),
/// the 4-byte `codec` FourCC (ADR-0016), a little-endian `u32` length prefix,
/// and the encoded bytes. The tag itself is written separately by the media
/// task, so this is only the source-tagged, codec-tagged, length-prefixed
/// payload.
///
/// Returns the framed bytes, or `None` if `payload` is larger than `u32::MAX`
/// (far beyond any real camera frame; a length that cannot be expressed in
/// the 4-byte prefix must not be silently truncated).
#[cfg(test)]
#[must_use]
pub fn frame_video_payload(source_id: u8, codec: [u8; 4], payload: &[u8]) -> Option<Vec<u8>> {
    let len = u32::try_from(payload.len()).ok()?;
    let mut out = Vec::with_capacity(1 + 4 + 4 + payload.len());
    out.push(source_id);
    out.extend_from_slice(&codec);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(payload);
    Some(out)
}

/// Parses a source-tagged, codec-tagged, length-prefixed video-stream body
/// produced by [`frame_video_payload`] (the bytes after the [`VIDEO_FRAME`]
/// tag), returning the `(source_id, fourcc, payload)` triple.
///
/// Returns `None` if fewer than the one source-id plus four FourCC plus four
/// length bytes are present or the declared length does not match the
/// remaining bytes exactly. Only the framing round-trip tests parse the
/// payload back host-side; the real client readers live in the native viewer
/// and the browser.
#[cfg(test)]
#[must_use]
pub fn parse_video_payload(body: &[u8]) -> Option<(u8, [u8; 4], &[u8])> {
    let (source_id, rest) = body.split_first()?;
    let (fourcc, rest) = rest.split_at_checked(4)?;
    let (len_bytes, rest) = rest.split_at_checked(4)?;
    let declared = u32::from_le_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]);
    let declared = usize::try_from(declared).ok()?;
    let fourcc = [fourcc[0], fourcc[1], fourcc[2], fourcc[3]];
    (rest.len() == declared).then_some((*source_id, fourcc, rest))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{
        FOURCC_MJPEG, VIDEO_FRAME, VIDEO_FRAME_V2, frame_video_payload, parse_video_payload,
    };

    #[test]
    fn v1_and_v2_kind_tags_are_distinct_and_stable() {
        assert_eq!(VIDEO_FRAME, 0x02, "v1 video kind byte keeps its meaning");
        assert_eq!(VIDEO_FRAME_V2, 0x03, "v2 video kind byte");
        assert_ne!(
            VIDEO_FRAME, VIDEO_FRAME_V2,
            "a reader can tell v1 and v2 apart"
        );
    }

    #[test]
    fn framing_round_trips() {
        let jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3, 0xFF, 0xD9];
        let framed = frame_video_payload(1, FOURCC_MJPEG, &jpeg).expect("small frame frames");
        assert_eq!(framed[0], 1, "leads with the source id");
        assert_eq!(&framed[1..5], b"MJPG");
        assert_eq!(&framed[5..9], &(jpeg.len() as u32).to_le_bytes());
        let (source_id, codec, parsed) = parse_video_payload(&framed).expect("body parses back");
        assert_eq!(source_id, 1);
        assert_eq!(codec, FOURCC_MJPEG);
        assert_eq!(parsed, jpeg.as_slice());
    }

    #[test]
    fn empty_jpeg_frames_and_parses() {
        let framed = frame_video_payload(0, FOURCC_MJPEG, &[]).expect("empty frames");
        assert_eq!(framed, vec![0, b'M', b'J', b'P', b'G', 0, 0, 0, 0]);
        assert_eq!(
            parse_video_payload(&framed),
            Some((0, FOURCC_MJPEG, &[][..]))
        );
    }

    #[test]
    fn truncated_header_is_rejected() {
        // Fewer than the 1-byte source id plus 8-byte fourcc+length header.
        assert_eq!(parse_video_payload(&[0, b'M', b'J', 0, 0]), None);
    }

    #[test]
    fn length_mismatch_is_rejected() {
        // Declares 5 bytes but only 2 follow.
        let body = vec![0, b'M', b'J', b'P', b'G', 5, 0, 0, 0, 1, 2];
        assert_eq!(parse_video_payload(&body), None);
    }

    use super::{VideoCaptureStamp, frame_video_payload_v2};
    use pilotage_adapter_api::{
        CalibrationId, CameraId, CaptureClockMapping, MeasurementClock, MeasurementStamp,
        SourceIncarnation, SourceIntegrity, SourceRole,
    };
    use pilotage_protocol::video_frame::META_LEN;

    fn le_u32(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    }

    fn le_u64(bytes: &[u8], at: usize) -> u64 {
        let mut buf = [0_u8; 8];
        buf.copy_from_slice(&bytes[at..at + 8]);
        u64::from_le_bytes(buf)
    }

    fn le_i64(bytes: &[u8], at: usize) -> i64 {
        let mut buf = [0_u8; 8];
        buf.copy_from_slice(&bytes[at..at + 8]);
        i64::from_le_bytes(buf)
    }

    fn sample_capture(mapping: CaptureClockMapping) -> VideoCaptureStamp {
        VideoCaptureStamp {
            stamp: MeasurementStamp {
                role: SourceRole::VideoCapture,
                integrity: SourceIntegrity::Unprotected,
                source_id: 1,
                source_incarnation: SourceIncarnation::new([0xAB; 16]),
                source_epoch: 7,
                sequence: 42,
                acquired_at_ns: 123_456,
                clock: MeasurementClock::Simulation,
            },
            camera_id: CameraId(9),
            calibration_id: CalibrationId(3),
            mapping,
        }
    }

    #[test]
    fn v2_header_places_every_field_at_its_offset() {
        let capture = sample_capture(CaptureClockMapping::Bounded {
            target: MeasurementClock::VehicleBoot,
            offset_ns: -1_000,
            error_bound_ns: 250,
        });
        let jpeg = vec![0xFF, 0xD8, 1, 2, 3, 0xFF, 0xD9];
        let body =
            frame_video_payload_v2(1, &capture, 5_000, 6_000, FOURCC_MJPEG, &jpeg).expect("frames");

        assert_eq!(body[0], 1, "source_id routing byte");
        assert_eq!(le_u32(&body, 1), 7, "source_epoch");
        assert_eq!(&body[5..21], &[0xAB; 16], "source_incarnation");
        assert_eq!(le_u32(&body, 21), 42, "sequence");
        assert_eq!(le_u64(&body, 25), 123_456, "capture_time_ns");
        assert_eq!(body[33], 2, "capture clock = simulation");
        assert_eq!(body[34], 1, "mapping available");
        assert_eq!(body[35], 1, "mapping target = vehicle boot");
        assert_eq!(le_i64(&body, 36), -1_000, "mapping offset");
        assert_eq!(le_u64(&body, 44), 250, "clock error bound");
        assert_eq!(le_u64(&body, 52), 5_000, "receive time");
        assert_eq!(le_u64(&body, 60), 6_000, "publication time");
        assert_eq!(le_u32(&body, 68), 9, "camera id");
        assert_eq!(le_u32(&body, 72), 3, "calibration id");
        assert_eq!(&body[76..80], b"MJPG", "codec fourcc after the header");
        assert_eq!(le_u32(&body, 80), jpeg.len() as u32, "length prefix");
        // Header (META_LEN) + 4-byte FourCC + 4-byte length prefix.
        assert_eq!(
            &body[META_LEN + 8..],
            jpeg.as_slice(),
            "payload trails intact"
        );
    }

    #[test]
    fn v2_unavailable_mapping_zeroes_the_mapping_fields() {
        let capture = sample_capture(CaptureClockMapping::Unavailable);
        let body =
            frame_video_payload_v2(0, &capture, 1, 2, FOURCC_MJPEG, &[9, 9]).expect("frames");
        assert_eq!(body[34], 0, "mapping unavailable flag");
        assert_eq!(body[35], 0, "no target clock");
        assert_eq!(le_i64(&body, 36), 0, "no offset");
        assert_eq!(le_u64(&body, 44), 0, "no error bound");
        // The capture stamp itself is still preserved.
        assert_eq!(le_u64(&body, 25), 123_456, "capture time survives");
    }
}
