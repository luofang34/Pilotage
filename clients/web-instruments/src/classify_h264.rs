//! Browser-facing H.264 access-unit classification, compiled from the same
//! [`pilotage_protocol::h264`] definitions every other consumer uses, so the
//! NAL-structure rules (keyframe, in-band SPS/PPS, codec string) can never
//! drift between the viewer and the wire's producers. The viewer keeps only
//! the WebCodecs session layer — decoder ownership, configure/feed, paint —
//! and asks this export what a chunk MEANS before acting on it.

use pilotage_protocol::h264::{ChunkClass, classify_chunk};
use serde::Serialize;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::wire_js::to_js;

/// One access unit's meaning, in the session layer's vocabulary: `kind` is
/// `"delta"`, `"keyframe"`, or `"undecodable-keyframe"`; `codec` is the
/// `avc1.PPCCLL` string on a decodable keyframe; `reason` is the typed fault
/// on an undecodable one.
#[derive(Serialize)]
struct ChunkClassJs {
    kind: &'static str,
    codec: Option<String>,
    reason: Option<&'static str>,
}

/// Classifies one H.264 Annex-B access unit (the codec payload of an `H264`
/// FourCC video body). Never throws: every input maps to a classification,
/// and malformed bytes classify as `"delta"`, which a session layer cannot
/// act on before a decodable keyframe.
#[wasm_bindgen(js_name = classifyH264Chunk)]
#[must_use]
pub fn classify_h264_chunk(payload: &[u8]) -> JsValue {
    let class = match classify_chunk(payload) {
        ChunkClass::Delta => ChunkClassJs {
            kind: "delta",
            codec: None,
            reason: None,
        },
        ChunkClass::Keyframe { codec } => ChunkClassJs {
            kind: "keyframe",
            codec: Some(codec),
            reason: None,
        },
        ChunkClass::UndecodableKeyframe { fault } => ChunkClassJs {
            kind: "undecodable-keyframe",
            codec: None,
            reason: Some(fault.reason()),
        },
    };
    to_js(&class)
}
