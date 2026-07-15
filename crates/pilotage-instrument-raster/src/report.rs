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
    /// Deterministic work performed producing this frame.
    pub work: RenderWork,
}

/// The target-independent work metric for one render: a pure function
/// of scene bytes and framebuffer dimensions, identical on every
/// platform, so engineering budgets can gate it in CI long before any
/// display hardware exists to time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RenderWork {
    /// Pixel-center coverage evaluations across all primitives.
    pub coverage_samples: u64,
    /// Worst-case per-edge/segment/disc tests inside those coverage
    /// evaluations — the unit a target timing model prices, since a
    /// polygon sample walks every edge while an arc sample tests one disc.
    pub edge_tests: u64,
    /// Source-over composites applied.
    pub composites: u64,
}

impl RenderWork {
    /// Engineering work budget for one panel frame.
    ///
    /// The fully populated PFD demo fixture — every band painting content —
    /// measures ~547k coverage samples, ~2.0M edge tests, and ~434k
    /// composites on the 480x360 panel (~3.2 samples and ~2.5 composites per
    /// output pixel). The budget grants 2x headroom over that worst measured
    /// fixture, rounded up, so scenes can grow denser without churning the
    /// constant while per-frame work stays bounded at ~6.4 samples per
    /// pixel. Because the counters are a pure function of scene bytes and
    /// dimensions, exceeding this budget is a deterministic CI failure on
    /// every platform, not a timing flake. The [`crate::timing`] model
    /// prices this budget into a worst-case execution time and gates it
    /// against the recorded frame deadline.
    pub const BUDGET: RenderWork = RenderWork {
        coverage_samples: 1_100_000,
        edge_tests: 4_000_000,
        composites: 900_000,
    };

    /// Whether this work fits inside `budget` on every axis.
    #[must_use]
    pub const fn within(&self, budget: &RenderWork) -> bool {
        self.coverage_samples <= budget.coverage_samples
            && self.edge_tests <= budget.edge_tests
            && self.composites <= budget.composites
    }
}
