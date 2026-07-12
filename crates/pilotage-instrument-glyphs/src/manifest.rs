//! The glyph manifest: metrics contract, lookup, and verification.

use crate::canonical::{
    CANONICAL_LEN, RECORDED_HASH, build_canonical, content_hash_of, glyph_count,
};
use crate::error::GlyphError;
use crate::font::GROUPS;
use crate::glyph::{
    ADVANCE, BASELINE, CELL_H, CELL_W, GLYPH_MANIFEST_VERSION, Glyph, GlyphId, GlyphRef,
};
use crate::vocabulary::PANEL_VOCABULARY;

/// A controlled glyph pack: fixed geometry plus an ordered set of glyphs,
/// pinned by a content hash.
///
/// The reference renderer and the browser backend share one manifest, so
/// they agree on glyph ids, advances, anchors, and the exact pixels. Lookup
/// and [`GlyphManifest::verify`] fail closed: a missing character or a hash
/// mismatch is an error, never a substituted or system glyph.
#[derive(Clone, Copy, Debug)]
pub struct GlyphManifest {
    groups: &'static [&'static [Glyph]],
}

/// The shipped glyph pack covering the panel and flag vocabularies.
pub const PANEL_GLYPHS: GlyphManifest = GlyphManifest { groups: GROUPS };

impl GlyphManifest {
    /// The manifest format version.
    pub const fn version(&self) -> u16 {
        GLYPH_MANIFEST_VERSION
    }

    /// Cell size as `(width, height)` in pixels.
    pub const fn cell(&self) -> (u8, u8) {
        (CELL_W as u8, CELL_H as u8)
    }

    /// Monospace horizontal advance, in cell columns.
    pub const fn advance(&self) -> u8 {
        ADVANCE
    }

    /// Rows from the cell top to the text baseline.
    pub const fn baseline(&self) -> u8 {
        BASELINE
    }

    /// Number of glyphs in the pack.
    pub fn len(&self) -> usize {
        glyph_count(self.groups)
    }

    /// Whether the pack has no glyphs.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iterates the glyphs in canonical order, assigning each its
    /// position-derived [`GlyphId`].
    pub fn iter(&self) -> impl Iterator<Item = GlyphRef> + '_ {
        self.groups
            .iter()
            .flat_map(|group| group.iter())
            .enumerate()
            .map(|(i, glyph)| GlyphRef {
                id: GlyphId(i as u16),
                glyph: *glyph,
            })
    }

    /// The glyph for `ch`, or `None` if the pack does not cover it.
    pub fn lookup(&self, ch: char) -> Option<GlyphRef> {
        self.iter().find(|entry| entry.glyph.ch == ch)
    }

    /// The glyph for `ch`, failing with [`GlyphError::MissingGlyph`] rather
    /// than substituting when the pack does not cover it.
    pub fn glyph(&self, ch: char) -> Result<GlyphRef, GlyphError> {
        self.lookup(ch).ok_or(GlyphError::MissingGlyph { ch })
    }

    /// The content hash recomputed from the live glyph data.
    pub fn content_hash(&self) -> [u8; 32] {
        content_hash_of(self.groups)
    }

    /// The hash recorded for the controlled pack at build time.
    pub const fn recorded_hash(&self) -> [u8; 32] {
        RECORDED_HASH
    }

    /// Byte length of this pack's canonical serialization.
    pub const fn canonical_len(&self) -> usize {
        CANONICAL_LEN
    }

    /// Writes the canonical serialization into `out`, returning its length,
    /// or [`GlyphError::BufferTooSmall`] if `out` cannot hold it.
    pub fn write_canonical(&self, out: &mut [u8]) -> Result<usize, GlyphError> {
        let bytes = build_canonical(self.groups);
        let dst = out
            .get_mut(..bytes.len())
            .ok_or(GlyphError::BufferTooSmall {
                needed: bytes.len(),
            })?;
        dst.copy_from_slice(&bytes);
        Ok(bytes.len())
    }

    /// Verifies the pack fully: geometry in range, every glyph well-formed,
    /// every mandatory character present, and the content hash matching the
    /// recorded one. Any failure returns the specific typed reason.
    pub fn verify(&self) -> Result<(), GlyphError> {
        self.check_geometry()?;
        self.check_glyphs()?;
        self.check_completeness()?;
        self.check_hash()
    }

    fn check_geometry(&self) -> Result<(), GlyphError> {
        let width_ok = CELL_W >= 1 && CELL_W <= 8;
        let height_ok = CELL_H >= 1 && CELL_H <= 8;
        let baseline_ok = BASELINE as usize <= CELL_H;
        if !width_ok || !height_ok || ADVANCE == 0 || !baseline_ok {
            return Err(GlyphError::InvalidGeometry);
        }
        Ok(())
    }

    fn check_glyphs(&self) -> Result<(), GlyphError> {
        let unused_mask: u8 = !(((1u16 << CELL_W) - 1) as u8);
        for entry in self.iter() {
            let g = entry.glyph;
            if g.advance == 0 {
                return Err(GlyphError::InvalidGlyph { ch: g.ch });
            }
            if g.rows.iter().any(|row| row & unused_mask != 0) {
                return Err(GlyphError::InvalidGlyph { ch: g.ch });
            }
        }
        Ok(())
    }

    fn check_completeness(&self) -> Result<(), GlyphError> {
        for &ch in PANEL_VOCABULARY {
            if self.lookup(ch).is_none() {
                return Err(GlyphError::MissingGlyph { ch });
            }
        }
        Ok(())
    }

    fn check_hash(&self) -> Result<(), GlyphError> {
        let computed = self.content_hash();
        if computed != RECORDED_HASH {
            return Err(GlyphError::ContentHashMismatch {
                computed,
                expected: RECORDED_HASH,
            });
        }
        Ok(())
    }

    /// Builds a manifest over an arbitrary group set for corruption tests;
    /// the group set must carry the shipped glyph count.
    #[cfg(test)]
    pub(crate) const fn from_groups(groups: &'static [&'static [Glyph]]) -> Self {
        Self { groups }
    }
}

#[cfg(test)]
mod tests;
