//! Glyph-cell geometry and the per-glyph bitmap record.
//!
//! Glyphs are authored as a fixed [`CELL_W`]×[`CELL_H`] pixel matrix. Each
//! row is one `u8`; the low [`CELL_W`] bits are pixels with bit
//! `CELL_W - 1` at the left column, so a row literal reads left-to-right as
//! the drawn shape (`0b01110` is `.###.`). Bits above [`CELL_W`] are unused
//! and must be zero; [`crate::GlyphManifest::verify`] rejects a glyph that
//! sets them.

/// Pixel columns in a glyph cell.
pub const CELL_W: usize = 5;

/// Pixel rows in a glyph cell.
pub const CELL_H: usize = 7;

/// Monospace horizontal advance, in cell columns (cell width plus one
/// column of inter-glyph spacing).
pub const ADVANCE: u8 = 6;

/// Rows from the cell top down to the text baseline. The pack has no
/// sub-baseline descenders, so the baseline sits at the bottom cell edge;
/// an outline-font upgrade would move it and bump the manifest version.
pub const BASELINE: u8 = CELL_H as u8;

/// Glyph-manifest format version, written into the hashed canonical form.
///
/// Growth is by appending glyphs; a geometry or field-layout change bumps
/// this and, with it, the recorded content hash.
pub const GLYPH_MANIFEST_VERSION: u16 = 1;

/// Stable numeric identifier for a glyph, assigned by canonical order.
///
/// Both the reference renderer and the browser backend walk the same
/// frozen glyph order, so an id denotes the same glyph on either side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct GlyphId(pub u16);

/// One glyph: its character, monospace advance, and pixel rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph {
    /// The Unicode scalar this glyph draws.
    pub ch: char,
    /// Horizontal advance in cell columns.
    pub advance: u8,
    /// Pixel rows, top to bottom; see the module docs for the bit layout.
    pub rows: [u8; CELL_H],
}

impl Glyph {
    /// A monospace glyph with the default [`ADVANCE`].
    pub const fn new(ch: char, rows: [u8; CELL_H]) -> Self {
        Self {
            ch,
            advance: ADVANCE,
            rows,
        }
    }

    /// Whether the pixel at `col`/`row` (0-based, from the top-left) is set.
    /// Coordinates outside the cell read as unset.
    pub const fn pixel(&self, col: usize, row: usize) -> bool {
        if col >= CELL_W || row >= CELL_H {
            return false;
        }
        let bit = CELL_W - 1 - col;
        (self.rows[row] >> bit) & 1 == 1
    }
}

/// A glyph paired with the [`GlyphId`] its canonical position assigns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlyphRef {
    /// The glyph's stable id.
    pub id: GlyphId,
    /// The glyph data.
    pub glyph: Glyph,
}
