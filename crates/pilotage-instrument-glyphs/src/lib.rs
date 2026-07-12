//! Controlled, reproducible glyph pack for instrument symbology (ADR-0017).
//!
//! Mandatory instrument text must not depend on installed system fonts. This
//! crate supplies a project-authored pixel font and a [`GlyphManifest`] that
//! pins it: fixed geometry, an ordered glyph set with stable ids, and a
//! content hash the build computes over a canonical serialization. The
//! reference renderer and the browser backend consume the one manifest, so
//! they agree on glyph ids, advances, anchors, and the exact pixels.
//!
//! The pack fails closed. [`GlyphManifest::glyph`] returns
//! [`GlyphError::MissingGlyph`] instead of substituting, and
//! [`GlyphManifest::verify`] rejects out-of-range geometry, a malformed
//! glyph, a missing mandatory character, or a content-hash mismatch — never
//! a system-font fallback.
//!
//! Text metrics stay backend-owned (ADR-0017): the manifest publishes
//! advance and baseline for backends that lay out by metric, but instrument
//! panels position text by anchor and do not depend on precise glyph
//! geometry. The font is a monospace 5×7 bitmap; a licensed outline font is
//! a later upgrade behind this same manifest contract, gated by a version
//! bump.
//!
//! The crate is `#![no_std]`: glyphs are static data and verification runs
//! over borrowed bytes with no allocator.

#![no_std]

#[cfg(test)]
extern crate std;

mod canonical;
mod error;
mod font;
mod glyph;
mod manifest;
mod sha256;
mod vocabulary;

pub use canonical::{CANONICAL_LEN, RECORDED_HASH};
pub use error::GlyphError;
pub use glyph::{
    ADVANCE, BASELINE, CELL_H, CELL_W, GLYPH_MANIFEST_VERSION, Glyph, GlyphId, GlyphRef,
};
pub use manifest::{GlyphManifest, PANEL_GLYPHS};
pub use sha256::sha256;
pub use vocabulary::{FLAG_VOCABULARY, PANEL_STRINGS, PANEL_VOCABULARY};
