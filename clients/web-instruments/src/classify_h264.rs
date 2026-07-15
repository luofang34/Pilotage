//! Browser-facing H.264 decode decisions, compiled from the same
//! [`pilotage_protocol::h264`] definitions every other consumer uses, so the
//! NAL-structure rules (keyframe, in-band SPS/PPS, codec string), the
//! decode-session state machine with its decoder generations, and per-source
//! decoder ownership can never drift between the viewer and the wire's
//! producers. The viewer keeps only what the browser platform forces —
//! executing the returned actions against WebCodecs `VideoDecoder`,
//! reporting platform failures back, and painting frames — and holds no
//! decision state of its own.

use pilotage_protocol::h264::{ChunkClass, classify_chunk};
use serde::Serialize;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::wire_js::to_js;

/// One access unit's meaning, in the session layer's vocabulary: `kind` is
/// `"invalid"`, `"delta"`, `"keyframe"`, or `"undecodable-keyframe"`;
/// `codec` is the `avc1.PPCCLL` string on a decodable keyframe; `reason` is
/// the typed fault on an invalid or undecodable one.
#[derive(Serialize)]
struct ChunkClassJs {
    kind: &'static str,
    codec: Option<String>,
    reason: Option<&'static str>,
}

/// Classifies one H.264 Annex-B access unit (the codec payload of an `H264`
/// FourCC video body). Never throws: every input maps to a classification.
/// Bytes with no NAL units classify as `"invalid"` with a typed reason — a
/// session layer fails closed on them rather than feeding a decoder.
#[wasm_bindgen(js_name = classifyH264Chunk)]
#[must_use]
pub fn classify_h264_chunk(payload: &[u8]) -> JsValue {
    let class = match classify_chunk(payload) {
        ChunkClass::Invalid => ChunkClassJs {
            kind: "invalid",
            codec: None,
            reason: Some("no Annex-B NAL units in the payload"),
        },
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

/// One decode-session decision, in the platform layer's vocabulary: `action`
/// is `"configure-and-feed"`, `"feed"`, `"drop"`, or `"fail"`; `codec` names
/// the configure target, `keyframe` the chunk type on a feed, `reason` the
/// typed fault on a fail.
#[derive(Serialize)]
struct FeedActionJs {
    action: &'static str,
    codec: Option<String>,
    keyframe: Option<bool>,
    reason: Option<&'static str>,
}

/// The decode-session state machine for one video source, compiled from
/// [`pilotage_protocol::h264::DecodeSession`]. The platform layer executes
/// the returned actions against WebCodecs and reports platform failures
/// back; it holds no decision state of its own.
#[wasm_bindgen]
pub struct H264DecodeSession {
    inner: pilotage_protocol::h264::DecodeSession,
}

#[wasm_bindgen]
impl H264DecodeSession {
    /// A fresh, unconfigured session.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: pilotage_protocol::h264::DecodeSession::new(),
        }
    }

    /// Classifies one access unit and returns the platform action.
    #[wasm_bindgen(js_name = onChunk)]
    #[must_use]
    pub fn on_chunk(&mut self, payload: &[u8]) -> JsValue {
        use pilotage_protocol::h264::FeedAction;
        let action = match self.inner.on_chunk(payload) {
            FeedAction::ConfigureAndFeed { codec } => FeedActionJs {
                action: "configure-and-feed",
                codec: Some(codec),
                keyframe: Some(true),
                reason: None,
            },
            FeedAction::Feed { keyframe } => FeedActionJs {
                action: "feed",
                codec: None,
                keyframe: Some(keyframe),
                reason: None,
            },
            FeedAction::Drop => FeedActionJs {
                action: "drop",
                codec: None,
                keyframe: None,
                reason: None,
            },
            FeedAction::Fail { reason } => FeedActionJs {
                action: "fail",
                codec: None,
                keyframe: None,
                reason: Some(reason),
            },
        };
        to_js(&action)
    }

    /// The platform decoder failed (configure error, decode throw, or the
    /// asynchronous error callback): the session fails permanently.
    #[wasm_bindgen(js_name = platformFailed)]
    pub fn platform_failed(&mut self) {
        self.inner.platform_failed();
    }

    /// Whether the session has failed and will feed nothing further.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn failed(&self) -> bool {
        self.inner.is_failed()
    }

    /// The current decoder generation; a platform callback captured at
    /// configure time is honored only while its captured value matches.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn generation(&self) -> u32 {
        self.inner.generation()
    }

    /// Whether a callback captured at `generation` may still deliver output.
    #[wasm_bindgen(js_name = acceptsOutputFrom)]
    #[must_use]
    pub fn accepts_output_from(&self, generation: u32) -> bool {
        self.inner.accepts_output_from(generation)
    }

    /// Retires the session permanently (its owner discarded it).
    pub fn retire(&mut self) {
        self.inner.retire();
    }
}

impl Default for H264DecodeSession {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-source decoder ownership bound to a numeric session token, compiled
/// from [`pilotage_protocol::h264::SourceOwnership`]. The platform layer
/// holds the decoder objects and executes the returned transition:
/// `"reuse"`, `"build"`, or `"replace"` (close the retired decoder first).
#[wasm_bindgen]
pub struct H264SourceOwnership {
    inner: pilotage_protocol::h264::SourceOwnership,
}

#[wasm_bindgen]
impl H264SourceOwnership {
    /// An empty ownership table.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: pilotage_protocol::h264::SourceOwnership::new(),
        }
    }

    /// Claims `source` for the session `token` and returns the transition.
    #[must_use]
    pub fn claim(&mut self, source: u32, token: u32) -> String {
        use pilotage_protocol::h264::ClaimAction;
        match self.inner.claim(source, u64::from(token)) {
            ClaimAction::Reuse => "reuse".to_string(),
            ClaimAction::Build => "build".to_string(),
            ClaimAction::Replace => "replace".to_string(),
        }
    }

    /// Drops `source`'s claim; `true` when a decoder was held and must close.
    pub fn reset(&mut self, source: u32) -> bool {
        self.inner.reset(source)
    }

    /// Drops every claim (session teardown).
    pub fn clear(&mut self) {
        self.inner.clear();
    }
}

impl Default for H264SourceOwnership {
    fn default() -> Self {
        Self::new()
    }
}
