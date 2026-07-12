//! Typed rasterizer failures.
//!
//! Geometry errors ([`RasterError::ZeroFramebuffer`] and the other
//! framebuffer-shape variants) are reported before the framebuffer is
//! touched, so the caller's buffer is left untouched. Every other error is
//! raised only after the framebuffer shape is accepted; the renderer then
//! spoils the whole frame (see [`crate::render`]) so no plausible old frame
//! survives a failure.

use pilotage_instrument_scene::{DecodeError, LayerError};

/// Why a render did not produce a valid frame.
#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq)]
pub enum RasterError {
    /// The framebuffer has a zero width or height.
    #[error("framebuffer width and height must be non-zero")]
    ZeroFramebuffer,
    /// A framebuffer dimension exceeds [`crate::MAX_DIMENSION`].
    #[error("framebuffer {width}x{height} exceeds the {limit}px dimension limit")]
    FramebufferTooLarge {
        /// Requested width in pixels.
        width: u32,
        /// Requested height in pixels.
        height: u32,
        /// The per-axis pixel limit.
        limit: u32,
    },
    /// The row stride is narrower than one packed RGBA8 row.
    #[error("row stride {stride_bytes} is below the {min_bytes}-byte minimum")]
    StrideTooSmall {
        /// The stride the caller supplied.
        stride_bytes: usize,
        /// Bytes one packed row needs (`width * 4`).
        min_bytes: usize,
    },
    /// The pixel slice is shorter than `height * stride`.
    #[error("framebuffer slice has {have} bytes but needs {need}")]
    FramebufferTooSmall {
        /// Bytes the geometry needs.
        need: usize,
        /// Bytes the slice holds.
        have: usize,
    },
    /// The scene failed the layered-scene contract before painting.
    #[error("scene layer contract violated: {0}")]
    Layer(#[from] LayerError),
    /// A command stream decode failed during painting.
    #[error("scene decode failed during paint: {0}")]
    Decode(#[from] DecodeError),
    /// A coordinate, transform component, or size was NaN or infinite.
    #[error("a coordinate or transform value was not finite")]
    NonFinite,
    /// A device coordinate fell outside the representable range.
    #[error("a device coordinate exceeded the +/-{limit_px}px range")]
    CoordinateOutOfRange {
        /// The per-axis device-coordinate limit in pixels.
        limit_px: f32,
    },
    /// A polyline or polygon carried more vertices than one command may
    /// paint.
    #[error("a shape has more than {limit} vertices")]
    TooManyVertices {
        /// The per-command vertex limit.
        limit: usize,
    },
    /// The graphics-state save stack exceeded its depth budget.
    #[error("graphics-state stack exceeded depth {limit}")]
    StackOverflow {
        /// The maximum save depth.
        limit: usize,
    },
    /// A restore was issued with no matching save.
    #[error("restore with no matching save")]
    UnbalancedRestore,
}
