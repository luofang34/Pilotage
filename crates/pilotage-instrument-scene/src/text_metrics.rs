//! Nominal text extents — the layout half of the text contract.
//!
//! Exact glyph ink stays backend-owned (ADR-0017): anchors describe
//! intent, and no panel positions individual glyphs. But a layout that
//! must CONTAIN text — a readout box whose digits may never paint
//! outside it (DISP-01/DISP-02) — needs extents every backend agrees
//! on. These functions are that agreement: a conforming backend
//! advances the pen [`NOMINAL_ADVANCE_RATIO`] of the run size per
//! character and paints no ink past [`nominal_text_ink_width`]. The
//! ratios are the glyph manifest's cell advance and cell width over its
//! cell height; REN-04 conformance holds the shipped backends to them.

/// Per-character pen advance as a fraction of the run size.
pub const NOMINAL_ADVANCE_RATIO: f32 = 6.0 / 7.0;

/// Painted glyph width as a fraction of the run size; the difference
/// from [`NOMINAL_ADVANCE_RATIO`] is the inter-character gap.
pub const NOMINAL_GLYPH_RATIO: f32 = 5.0 / 7.0;

/// Anchor width of a run: `chars` advances at `size`. This is the
/// width backends use to place `Center` and `Right` anchors.
#[must_use]
pub fn nominal_text_width(size: f32, chars: usize) -> f32 {
    chars as f32 * size * NOMINAL_ADVANCE_RATIO
}

/// Painted (ink) width of a run: the anchor width minus the trailing
/// inter-character gap after the last glyph. Zero for an empty run.
#[must_use]
pub fn nominal_text_ink_width(size: f32, chars: usize) -> f32 {
    if chars == 0 {
        return 0.0;
    }
    nominal_text_width(size, chars) - size * (NOMINAL_ADVANCE_RATIO - NOMINAL_GLYPH_RATIO)
}
