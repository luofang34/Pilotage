//! Decode-session and decoder-ownership state machines for the `H264` video
//! path. The platform decoder (WebCodecs in the browser, or any native
//! decoder) holds no decision state: every chunk is classified
//! and every state transition decided here, and the platform layer only
//! executes the returned action — configure, feed, drop, or fail. A failed
//! session stays failed (one typed reason, then silence) until it is
//! discarded, so a platform callback can never resurrect it.

use std::collections::BTreeMap;

use super::{ChunkClass, classify_chunk};

/// What the platform layer must do with one access unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedAction {
    /// Close any held platform decoder, configure a fresh one for `codec`,
    /// then feed this chunk as a keyframe.
    ConfigureAndFeed {
        /// The `avc1.PPCCLL` codec string to configure with.
        codec: String,
    },
    /// Feed this chunk to the already-configured platform decoder.
    Feed {
        /// Whether the chunk is a keyframe (`true`) or a delta (`false`).
        keyframe: bool,
    },
    /// Drop this chunk: no decoder is configured and the chunk cannot start
    /// one (a delta before the first decodable keyframe).
    Drop,
    /// The session failed on this chunk; surface `reason` once, feed nothing,
    /// and never feed again.
    Fail {
        /// The typed reason the session cannot decode.
        reason: &'static str,
    },
}

/// The decode-session state machine for one video source: unconfigured until
/// a decodable keyframe arrives, configured for that keyframe's codec until
/// an in-band codec change reconfigures it, failed — permanently — on
/// invalid input, an undecodable keyframe, or a platform CAPABILITY failure
/// (no decoder, configure error), and retired when its owner discards it.
/// A platform-reported DECODE error is different: it is a mid-stream data
/// fault (a lost access unit leaves the next delta referencing a missing
/// frame), so the session re-enters unconfigured and awaits the next
/// decodable keyframe instead of dying — bounded by a strike count that only
/// a subsequently painted output frame re-arms, so a stream that never
/// recovers still fails closed. Each configuration is a
/// distinct decoder GENERATION: a platform callback captured at configure
/// time carries its generation and is honored only while it matches
/// [`DecodeSession::generation`], so a callback from a replaced, reset, or
/// failed decoder can never paint over its successor.
#[derive(Debug, Default)]
pub struct DecodeSession {
    configured_codec: Option<String>,
    generation: u32,
    failed: bool,
    retired: bool,
    decode_error_strikes: u8,
}

/// How the session absorbed a platform-reported decode error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeErrorRecovery {
    /// The session dropped to unconfigured: the platform layer closes its
    /// decoder, and the next decodable keyframe reconfigures a fresh one.
    AwaitKeyframe,
    /// The strike bound is exhausted (or the session was already dead): the
    /// failure is permanent and surfaced once.
    Failed,
}

/// Decode-error recoveries allowed without an intervening painted output
/// frame. Exhausting them fails the session closed: a source whose every
/// reconfiguration errors again is broken at the stream, not the viewer.
const MAX_DECODE_ERROR_STRIKES: u8 = 3;

impl DecodeSession {
    /// A fresh, unconfigured session.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Classifies one access unit and decides the platform action. The
    /// decision is final for this chunk: a failed or retired session only
    /// ever drops, and invalid input or an undecodable keyframe fails the
    /// session exactly once.
    pub fn on_chunk(&mut self, payload: &[u8]) -> FeedAction {
        if self.failed || self.retired {
            return FeedAction::Drop;
        }
        match classify_chunk(payload) {
            ChunkClass::Invalid => {
                self.fail_now();
                FeedAction::Fail {
                    reason: "no Annex-B NAL units in the payload",
                }
            }
            ChunkClass::UndecodableKeyframe { fault } => {
                self.fail_now();
                FeedAction::Fail {
                    reason: fault.reason(),
                }
            }
            ChunkClass::Keyframe { codec } => {
                if self.configured_codec.as_deref() == Some(codec.as_str()) {
                    FeedAction::Feed { keyframe: true }
                } else {
                    self.configured_codec = Some(codec.clone());
                    self.generation = self.generation.wrapping_add(1);
                    FeedAction::ConfigureAndFeed { codec }
                }
            }
            ChunkClass::Delta => {
                if self.configured_codec.is_some() {
                    FeedAction::Feed { keyframe: false }
                } else {
                    FeedAction::Drop
                }
            }
        }
    }

    /// The current decoder generation. A platform callback captured at
    /// configure time is honored only while its captured value matches; any
    /// reconfiguration or failure advances it, retiring stale callbacks.
    #[must_use]
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Whether a callback captured at `generation` may still deliver output:
    /// the session is live and no reconfiguration has superseded it.
    #[must_use]
    pub fn accepts_output_from(&self, generation: u32) -> bool {
        !self.failed && !self.retired && self.generation == generation
    }

    /// Retires the session (its owner discarded it — session replacement,
    /// capture discontinuity, teardown). Retired is permanent: every later
    /// chunk drops and every captured callback is refused.
    pub fn retire(&mut self) {
        self.retired = true;
        self.configured_codec = None;
        self.generation = self.generation.wrapping_add(1);
    }

    fn fail_now(&mut self) {
        self.failed = true;
        self.configured_codec = None;
        self.generation = self.generation.wrapping_add(1);
    }

    /// The platform decoder could not be built or configured — a CAPABILITY
    /// failure: the session fails permanently and stale callbacks are
    /// retired with it. A mid-stream decode error is not this; report it
    /// through [`Self::platform_decode_error`].
    pub fn platform_failed(&mut self) {
        self.fail_now();
    }

    /// The platform decoder reported a DECODE error (synchronous throw or
    /// the asynchronous error callback): a mid-stream data fault. Within the
    /// strike bound the session re-enters unconfigured — stale callbacks are
    /// retired by the generation advance, deltas drop, and the next
    /// decodable keyframe reconfigures. Beyond the bound it fails closed.
    pub fn platform_decode_error(&mut self) -> DecodeErrorRecovery {
        if self.failed || self.retired {
            return DecodeErrorRecovery::Failed;
        }
        self.decode_error_strikes = self.decode_error_strikes.saturating_add(1);
        if self.decode_error_strikes > MAX_DECODE_ERROR_STRIKES {
            self.fail_now();
            return DecodeErrorRecovery::Failed;
        }
        self.configured_codec = None;
        self.generation = self.generation.wrapping_add(1);
        DecodeErrorRecovery::AwaitKeyframe
    }

    /// The platform painted an output frame from `generation`: the stream is
    /// demonstrably decoding again, so the decode-error strike count
    /// re-arms. A stale generation proves nothing and is ignored.
    pub fn note_output(&mut self, generation: u32) {
        if self.accepts_output_from(generation) {
            self.decode_error_strikes = 0;
        }
    }

    /// Whether the session has failed and will feed nothing further.
    #[must_use]
    pub fn is_failed(&self) -> bool {
        self.failed
    }

    /// Whether a decoder is currently configured.
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.configured_codec.is_some()
    }
}

/// How the platform layer must serve a `(source, token)` claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimAction {
    /// The held decoder belongs to this session token: reuse it.
    Reuse,
    /// No decoder is held for the source: build one bound to this token.
    Build,
    /// A decoder from a different session token is held: close it, then
    /// build a fresh one bound to this token — a retired token can never
    /// govern a live session's frames.
    Replace,
}

/// Per-source decoder ownership bound to a session token (the transport
/// session's numeric generation). This is the ownership boundary that keeps a
/// decoder from outliving its session; the platform layer holds the decoder
/// objects and executes the returned transitions.
#[derive(Debug, Default)]
pub struct SourceOwnership {
    tokens: BTreeMap<u32, u64>,
}

impl SourceOwnership {
    /// An empty ownership table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Claims `source` for the session `token` and returns the transition
    /// the platform layer must perform. The table records the claim.
    pub fn claim(&mut self, source: u32, token: u64) -> ClaimAction {
        match self.tokens.insert(source, token) {
            Some(held) if held == token => ClaimAction::Reuse,
            Some(_) => ClaimAction::Replace,
            None => ClaimAction::Build,
        }
    }

    /// Drops `source`'s claim (a capture discontinuity is a GOP boundary a
    /// decoder cannot span). Returns whether a decoder was held, i.e. whether
    /// the platform layer must close one.
    pub fn reset(&mut self, source: u32) -> bool {
        self.tokens.remove(&source).is_some()
    }

    /// Drops every claim (session teardown). The platform layer closes every
    /// held decoder.
    pub fn clear(&mut self) {
        self.tokens.clear();
    }
}
