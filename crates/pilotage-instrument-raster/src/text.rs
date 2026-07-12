//! The pre-glyph-pack text contract: a deterministic placeholder box.
//!
//! Until REN-02's glyph pack lands, a [`Cmd::Text`](pilotage_instrument_scene::Cmd::Text)
//! run renders as a stroked bounding box whose extent is a pure function of
//! the run's `size`, `anchor`, and UTF-8 byte length — never of any platform
//! font metrics — so frame hashes stay stable and swap cleanly for real
//! glyphs later. The box is stroked in the current fill color (text paints
//! with the fill color) at a fixed width; an empty run or a non-positive
//! size paints nothing.

use pilotage_instrument_scene::{Anchor, HAlign, VAlign};

use crate::error::RasterError;
use crate::stroke::stroke_path;
use crate::surface::{PixelRect, Surface};
use crate::transform::Affine;

/// Logical advance width per UTF-8 byte, as a fraction of the run size.
const ADVANCE_RATIO: f32 = 0.6;

/// Logical box height above the anchored baseline, as a fraction of size.
const ASCENT_RATIO: f32 = 0.75;

/// Logical box height below the baseline, as a fraction of size.
const DESCENT_RATIO: f32 = 0.25;

/// Stroke width of the placeholder box, in device pixels.
const BOX_STROKE: f32 = 1.0;

/// Renders the placeholder box for a text run using the current fill color.
pub(crate) fn draw_placeholder(
    surface: &mut Surface<'_>,
    clip: PixelRect,
    ctm: &Affine,
    run: Run,
    color: [u8; 4],
) -> Result<(), RasterError> {
    if run.byte_len == 0 || run.size <= 0.0 {
        return Ok(());
    }
    let w = run.byte_len as f32 * ADVANCE_RATIO * run.size;
    let left = match run.anchor.h {
        HAlign::Left => run.x,
        HAlign::Center => run.x - w / 2.0,
        HAlign::Right => run.x - w,
    };
    let (top, bottom) = match run.anchor.v {
        VAlign::Baseline => (
            run.y - ASCENT_RATIO * run.size,
            run.y + DESCENT_RATIO * run.size,
        ),
        VAlign::Top => (run.y, run.y + run.size),
        VAlign::Bottom => (run.y - run.size, run.y),
        VAlign::Middle => (run.y - run.size / 2.0, run.y + run.size / 2.0),
    };
    let right = left + w;
    let corners = [
        ctm.map(left, top)?,
        ctm.map(right, top)?,
        ctm.map(right, bottom)?,
        ctm.map(left, bottom)?,
    ];
    stroke_path(surface, clip, &corners, true, BOX_STROKE, color);
    Ok(())
}

/// A text run's placeholder-relevant fields.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Run {
    /// Anchor point x in logical units.
    pub(crate) x: f32,
    /// Anchor point y in logical units.
    pub(crate) y: f32,
    /// Run size in logical units.
    pub(crate) size: f32,
    /// Text positioning.
    pub(crate) anchor: Anchor,
    /// UTF-8 byte length of the run.
    pub(crate) byte_len: usize,
}

#[cfg(test)]
mod tests;
