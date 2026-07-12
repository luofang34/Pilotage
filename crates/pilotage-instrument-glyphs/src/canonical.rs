//! The canonical serialization the content hash is taken over.
//!
//! The canonical form is a fixed little-endian byte layout with no
//! timestamps, padding, or platform-dependent ordering, so the same glyph
//! data always produces the same bytes and therefore the same hash. The
//! layout is:
//!
//! - header (8 bytes): manifest version (u16), cell width, cell height,
//!   advance, baseline (u8 each), glyph count (u16);
//! - then, per glyph in [`crate::font`] order: character (u32), advance
//!   (u8), and the seven bitmap rows.
//!
//! [`RECORDED_HASH`] is [`crate::sha256::sha256`] of this form, evaluated at
//! compile time over the shipped glyphs, so verification compares live data
//! against a hash the build itself produced.

use crate::font::GROUPS;
use crate::glyph::{ADVANCE, BASELINE, CELL_H, CELL_W, GLYPH_MANIFEST_VERSION, Glyph};
use crate::sha256::sha256;

/// Fixed header length in bytes.
pub const HEADER_LEN: usize = 8;

/// Per-glyph record length in bytes: character, advance, and bitmap rows.
pub const PER_GLYPH_LEN: usize = 4 + 1 + CELL_H;

/// Total glyphs across all classes in `groups`.
pub const fn glyph_count(groups: &[&[Glyph]]) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < groups.len() {
        n += groups[i].len();
        i += 1;
    }
    n
}

/// Number of glyphs the shipped pack carries.
pub const GLYPH_COUNT: usize = glyph_count(GROUPS);

/// Byte length of the canonical serialization of the shipped pack.
pub const CANONICAL_LEN: usize = HEADER_LEN + PER_GLYPH_LEN * GLYPH_COUNT;

/// Serializes `groups` into the canonical byte layout.
///
/// The returned array is sized for the shipped [`GLYPH_COUNT`]; callers pass
/// a group set with that same total (the shipped pack, or a corruption of
/// it under test).
pub const fn build_canonical(groups: &[&[Glyph]]) -> [u8; CANONICAL_LEN] {
    let mut out = [0u8; CANONICAL_LEN];
    let ver = GLYPH_MANIFEST_VERSION.to_le_bytes();
    out[0] = ver[0];
    out[1] = ver[1];
    out[2] = CELL_W as u8;
    out[3] = CELL_H as u8;
    out[4] = ADVANCE;
    out[5] = BASELINE;
    let count = (glyph_count(groups) as u16).to_le_bytes();
    out[6] = count[0];
    out[7] = count[1];

    let mut pos = HEADER_LEN;
    let mut gi = 0;
    while gi < groups.len() {
        let group = groups[gi];
        let mut ci = 0;
        while ci < group.len() {
            let g = group[ci];
            let ch = (g.ch as u32).to_le_bytes();
            out[pos] = ch[0];
            out[pos + 1] = ch[1];
            out[pos + 2] = ch[2];
            out[pos + 3] = ch[3];
            out[pos + 4] = g.advance;
            let mut r = 0;
            while r < CELL_H {
                out[pos + 5 + r] = g.rows[r];
                r += 1;
            }
            pos += PER_GLYPH_LEN;
            ci += 1;
        }
        gi += 1;
    }
    out
}

/// The content hash of `groups`' canonical form.
pub const fn content_hash_of(groups: &[&[Glyph]]) -> [u8; 32] {
    sha256(&build_canonical(groups))
}

/// The content hash recorded for the shipped glyph pack, computed at compile
/// time from the canonical form.
pub const RECORDED_HASH: [u8; 32] = content_hash_of(GROUPS);
