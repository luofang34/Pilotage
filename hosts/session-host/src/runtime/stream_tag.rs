//! Kind tags that disambiguate the host's several host-initiated
//! unidirectional streams (ADR-0005).
//!
//! Both authority events and video frames travel on host-initiated uni
//! streams, so a reader accepting a fresh uni stream cannot tell them apart
//! from the QUIC stream type alone. Every host-initiated uni stream therefore
//! begins with a single kind-tag byte before its payload; the reader consumes
//! that byte first and routes on it.

/// Tag prefixing the dedicated authority-events stream: the reliable, ordered
/// stream of lease/handover/override events (ADR-0005) carries
/// length-delimited envelopes after this byte.
pub const AUTHORITY_EVENTS: u8 = 0x01;

/// Tag prefixing a per-frame video stream: a
/// `[source_id: u8][fourcc: 4 bytes][u32 LE len][payload]` media unit follows
/// this byte (ADR-0005 media = one uni stream per frame; ADR-0016 codec tag).
/// `source_id` names the video source (0 = onboard FPV, 1 = chase) so a client
/// with several sources routes each frame to the right one.
pub const VIDEO_FRAME: u8 = 0x02;

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
    use super::{FOURCC_MJPEG, frame_video_payload, parse_video_payload};

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
}
