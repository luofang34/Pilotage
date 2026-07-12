//! Framebuffer description and the stateless render report.

/// Pixel geometry of a caller-provided RGBA8 framebuffer.
///
/// Pixels are row-major, top-left origin, four bytes each (R, G, B, A),
/// with `stride_bytes` between the starts of consecutive rows so callers
/// can render into a sub-window of a larger buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramebufferDims {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Bytes between the starts of consecutive rows (`>= width * 4`).
    pub stride_bytes: u32,
}

impl FramebufferDims {
    /// Dimensions for a tightly packed buffer (`stride = width * 4`).
    pub const fn tight(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            stride_bytes: width.saturating_mul(4),
        }
    }
}

/// Opaque generation counters echoed back so a caller can correlate a
/// frame with the render that produced it.
///
/// The rasterizer is stateless: it neither reads nor advances these, it
/// only carries them from input to [`RenderReport`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameId {
    /// The snapshot/frame generation the scene was built from.
    pub frame_generation: u32,
    /// The render generation of this paint attempt.
    pub render_generation: u32,
}

/// Outcome discriminant of a completed render.
///
/// One success variant today; kept an enum so later outcomes (for example
/// a partial-content status) append without changing the success path's
/// meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderStatus {
    /// The full scene painted and self-validated.
    Painted,
}

/// What a successful render produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderReport {
    /// The scene format version byte that was honored.
    pub scene_version: u8,
    /// The completion status.
    pub status: RenderStatus,
    /// The echoed frame/render correlation identifiers.
    pub frame: FrameId,
    /// Unknown opcodes skipped during painting (version policy).
    pub unknown_opcodes: u32,
    /// Bitset of layers present, bit `i` for the layer with discriminant `i`.
    pub layers_present: u8,
}
