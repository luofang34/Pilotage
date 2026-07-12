//! The project-authored 5×7 pixel font.
//!
//! Every glyph is defined here as an original bit matrix (no third-party
//! font data); see `docs/instruments/glyph-pack.md` for the authorship and
//! license statement. Glyphs are grouped by class, and [`GROUPS`] fixes the
//! order in which the classes concatenate. Glyph ids and the hashed
//! canonical form both follow that order, so it must not be reshuffled
//! without a manifest-version bump.

mod digits;
mod letters_lower;
mod letters_upper;
mod symbols;

use crate::glyph::Glyph;

#[cfg(test)]
pub(crate) use digits::DIGITS_ARR;
#[cfg(test)]
pub(crate) use letters_lower::LOWER_ARR;
#[cfg(test)]
pub(crate) use letters_upper::UPPER_ARR;
#[cfg(test)]
pub(crate) use symbols::SYMBOLS_ARR;

use digits::DIGITS;
use letters_lower::LOWER;
use letters_upper::UPPER;
use symbols::SYMBOLS;

/// The frozen concatenation order of glyph classes.
///
/// Ids are assigned by walking these slices front to back, so appending a
/// glyph to a class shifts only the ids after it; reordering classes or
/// glyphs changes every following id and the content hash.
pub(crate) const GROUPS: &[&[Glyph]] = &[SYMBOLS, DIGITS, UPPER, LOWER];
