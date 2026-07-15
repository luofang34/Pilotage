//! Browser-facing decode of the `VIDEO_FRAME_V2` body, compiled from the same
//! [`pilotage_protocol::video_frame`] definition the host encodes with, so the
//! header layout and the capture-identity contract can never drift from the
//! producer (ADR-0020).
//!
//! The export returns the capture-identity metadata (in the exact shape the
//! browser identity gate consumes), the codec FourCC, the payload's offset and
//! length within the body (the JPEG/Annex-B bytes stay in the caller's buffer,
//! never copied out through wasm), and a typed `fault` when the header violates
//! the encoder contract. A structurally malformed body (too short, or a
//! length prefix that disagrees with the trailing bytes) decodes to `null`.

use pilotage_protocol::video_frame::{self, CaptureHeader, OFFSET};
use serde::Serialize;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::wire_js::{fourcc_string, incarnation_hex, to_js};

/// The capture-identity header in the browser gate's field vocabulary. `u64`
/// and `i64` fields serialize to `BigInt`; the rest to `Number`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VideoMeta {
    source_id: u8,
    source_epoch: u32,
    source_incarnation: String,
    sequence: u32,
    capture_time_nanos: u64,
    capture_clock: u8,
    mapping_available: bool,
    mapping_target_clock: u8,
    mapping_offset_nanos: i64,
    clock_error_bound_nanos: u64,
    receive_time_nanos: u64,
    publication_time_nanos: u64,
    camera_id: u32,
    calibration_id: u32,
}

/// A capture-identity contract violation, surfaced to the caller as the same
/// `{ field, rule }` reason the browser validator produced.
#[derive(Serialize)]
struct Fault {
    field: &'static str,
    rule: &'static str,
}

/// A decoded v2 video body: metadata, codec, the payload's position in the
/// caller's body buffer, and any contract fault.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DecodedVideoFrame {
    meta: VideoMeta,
    fourcc: String,
    payload_offset: u32,
    payload_len: u32,
    fault: Option<Fault>,
}

impl From<&CaptureHeader> for VideoMeta {
    fn from(header: &CaptureHeader) -> Self {
        Self {
            source_id: header.source_id,
            source_epoch: header.source_epoch,
            source_incarnation: incarnation_hex(&header.source_incarnation),
            sequence: header.sequence,
            capture_time_nanos: header.capture_time_ns,
            capture_clock: header.capture_clock,
            mapping_available: header.mapping_available,
            mapping_target_clock: header.mapping_target_clock,
            mapping_offset_nanos: header.mapping_offset_ns,
            clock_error_bound_nanos: header.clock_error_bound_ns,
            receive_time_nanos: header.receive_time_ns,
            publication_time_nanos: header.publication_time_ns,
            camera_id: header.camera_id,
            calibration_id: header.calibration_id,
        }
    }
}

/// Decodes a v2 video-frame body (the bytes after the kind tag). Returns
/// `null` for a structurally malformed body; otherwise an object with `meta`,
/// `fourcc`, `payloadOffset`, `payloadLen`, and a `fault` that is `null` when
/// the header satisfies the capture-identity contract or `{ field, rule }`
/// when it does not. The payload bytes are left in the caller's `body` buffer;
/// the caller slices `body[payloadOffset .. payloadOffset + payloadLen]`.
#[wasm_bindgen(js_name = decodeVideoFrameV2)]
#[must_use]
pub fn decode_video_frame_v2(body: &[u8]) -> JsValue {
    let Ok(frame) = video_frame::decode_v2(body) else {
        return JsValue::NULL;
    };
    // The payload offset is the fixed header + FourCC + length prefix; its
    // length is what the prefix declared (already checked against the body).
    let Ok(payload_len) = u32::try_from(frame.payload.len()) else {
        return JsValue::NULL;
    };
    let decoded = DecodedVideoFrame {
        meta: VideoMeta::from(&frame.header),
        fourcc: fourcc_string(frame.codec),
        payload_offset: OFFSET.payload as u32,
        payload_len,
        fault: frame.header.contract_fault().map(|fault| Fault {
            field: fault.field,
            rule: fault.rule,
        }),
    };
    to_js(&decoded)
}
