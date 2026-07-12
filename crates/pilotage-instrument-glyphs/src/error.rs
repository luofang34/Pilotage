//! The typed failure model for glyph lookup and manifest verification.

/// Why a glyph lookup or a [`crate::GlyphManifest::verify`] check failed.
///
/// Every variant is a fail-closed outcome: a missing or corrupt glyph pack
/// yields an error, never a substituted or system glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GlyphError {
    /// A required character has no glyph in the pack.
    #[error("mandatory glyph missing for character {ch:?}")]
    MissingGlyph {
        /// The character that was not found.
        ch: char,
    },
    /// A glyph's advance is zero or its bitmap sets pixels outside the cell.
    #[error("glyph for {ch:?} has a zero advance or out-of-cell pixels")]
    InvalidGlyph {
        /// The offending glyph's character.
        ch: char,
    },
    /// The manifest's geometry (cell size, advance, baseline) is out of the
    /// representable range.
    #[error("glyph manifest geometry is out of range")]
    InvalidGeometry,
    /// The recomputed content hash does not match the recorded one, so the
    /// loaded pack is not the controlled asset.
    #[error("glyph pack content hash mismatch")]
    ContentHashMismatch {
        /// Hash recomputed from the loaded data.
        computed: [u8; 32],
        /// Hash recorded for the controlled pack.
        expected: [u8; 32],
    },
    /// A caller buffer is too small to hold the canonical serialization.
    #[error("canonical buffer too small; need {needed} bytes")]
    BufferTooSmall {
        /// Bytes the canonical form requires.
        needed: usize,
    },
}
