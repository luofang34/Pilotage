//! Text through the controlled glyph pack (REN-02's contract).
//!
//! Each character renders from the verified bitmap manifest in
//! `pilotage-instrument-glyphs`: every set cell pixel becomes a logical
//! quad scaled by the run size and transformed like any other geometry,
//! so text obeys the same quantization boundary, clipping, and
//! compositing rules as primitives, with no platform font API anywhere.
//! A character without a glyph is a typed failure — nothing substitutes.
//! An empty run or a non-positive size paints nothing.
//!
//! Metrics are the manifest's: the run `size` maps to the cell height,
//! the pen advances by the manifest advance, and the bitmap sits
//! entirely above its baseline (descent zero), so `Bottom` anchors
//! coincide with `Baseline`.

use pilotage_instrument_glyphs::{ADVANCE, CELL_H, CELL_W, PANEL_GLYPHS};
use pilotage_instrument_scene::{Anchor, HAlign, VAlign};

use crate::error::RasterError;
use crate::paint::fill_polygon;
use crate::surface::{PixelRect, Surface};
use crate::transform::Affine;

/// Renders a text run from the glyph pack using the current fill color.
pub(crate) fn draw_run(
    surface: &mut Surface<'_>,
    clip: PixelRect,
    ctm: &Affine,
    run: Run<'_>,
    color: [u8; 4],
) -> Result<(), RasterError> {
    if run.text.is_empty() || run.size <= 0.0 {
        return Ok(());
    }
    let scale = run.size / CELL_H as f32;
    let advance = f32::from(ADVANCE) * scale;
    let width = run.text.chars().count() as f32 * advance;
    let left = match run.anchor.h {
        HAlign::Left => run.x,
        HAlign::Center => run.x - width / 2.0,
        HAlign::Right => run.x - width,
    };
    let top = match run.anchor.v {
        VAlign::Baseline | VAlign::Bottom => run.y - run.size,
        VAlign::Top => run.y,
        VAlign::Middle => run.y - run.size / 2.0,
    };
    let mut pen = left;
    for ch in run.text.chars() {
        let glyph = PANEL_GLYPHS.glyph(ch)?.glyph;
        for row in 0..CELL_H {
            for col in 0..CELL_W {
                if !glyph.pixel(col, row) {
                    continue;
                }
                let x0 = pen + col as f32 * scale;
                let y0 = top + row as f32 * scale;
                let quad = [
                    ctm.map(x0, y0)?,
                    ctm.map(x0 + scale, y0)?,
                    ctm.map(x0 + scale, y0 + scale)?,
                    ctm.map(x0, y0 + scale)?,
                ];
                fill_polygon(surface, clip, &quad, color);
            }
        }
        pen += advance;
    }
    Ok(())
}

/// A text run.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Run<'a> {
    /// Anchor point x in logical units.
    pub(crate) x: f32,
    /// Anchor point y in logical units.
    pub(crate) y: f32,
    /// Run size in logical units (maps to the glyph cell height).
    pub(crate) size: f32,
    /// Text positioning.
    pub(crate) anchor: Anchor,
    /// The UTF-8 text.
    pub(crate) text: &'a str,
}

#[cfg(test)]
mod tests;
