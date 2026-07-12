//! Manifest contract tests: completeness, reproducibility, fail-closed
//! lookup, and corruption detection.

#![allow(clippy::expect_used, clippy::panic)]

use crate::canonical::{CANONICAL_LEN, RECORDED_HASH};
use crate::error::GlyphError;
use crate::glyph::{CELL_H, CELL_W, Glyph};
use crate::manifest::{GlyphManifest, PANEL_GLYPHS};
use crate::vocabulary::{FLAG_VOCABULARY, PANEL_STRINGS, PANEL_VOCABULARY};
use std::collections::BTreeSet;
use std::vec::Vec;

#[test]
fn shipped_pack_verifies() {
    assert_eq!(PANEL_GLYPHS.verify(), Ok(()));
}

#[test]
fn panel_vocabulary_is_complete() {
    for &ch in PANEL_VOCABULARY {
        assert!(
            PANEL_GLYPHS.lookup(ch).is_some(),
            "panel vocabulary character {ch:?} has no glyph"
        );
    }
}

#[test]
fn panel_strings_resolve() {
    for label in PANEL_STRINGS {
        for ch in label.chars() {
            assert!(
                PANEL_GLYPHS.lookup(ch).is_some(),
                "panel label {label:?} has no glyph for {ch:?}"
            );
        }
    }
}

#[test]
fn flag_vocabulary_resolves() {
    for label in FLAG_VOCABULARY {
        for ch in label.chars() {
            assert!(
                PANEL_GLYPHS.lookup(ch).is_some(),
                "flag label {label:?} has no glyph for {ch:?}"
            );
        }
    }
}

#[test]
fn full_alphabet_and_digits_provided() {
    for ch in ('A'..='Z').chain('a'..='z').chain('0'..='9') {
        assert!(
            PANEL_GLYPHS.lookup(ch).is_some(),
            "expected a glyph for {ch:?}"
        );
    }
}

#[test]
fn missing_glyph_is_typed_error_never_substituted() {
    for ch in ['?', '#', '@', '\u{2603}'] {
        assert_eq!(PANEL_GLYPHS.lookup(ch), None);
        assert_eq!(PANEL_GLYPHS.glyph(ch), Err(GlyphError::MissingGlyph { ch }));
    }
}

#[test]
fn ids_are_dense_and_stable() {
    let ids: Vec<u16> = PANEL_GLYPHS.iter().map(|entry| entry.id.0).collect();
    let expected: Vec<u16> = (0..PANEL_GLYPHS.len() as u16).collect();
    assert_eq!(ids, expected);
    // The first canonical glyph is the space; lookup agrees with iteration.
    let space = PANEL_GLYPHS.glyph(' ').expect("space glyph present");
    assert_eq!(space.id.0, 0);
}

#[test]
fn no_duplicate_characters() {
    let mut seen = BTreeSet::new();
    for entry in PANEL_GLYPHS.iter() {
        assert!(
            seen.insert(entry.glyph.ch),
            "duplicate glyph for {:?}",
            entry.glyph.ch
        );
    }
}

#[test]
fn every_glyph_has_valid_metrics() {
    let unused_mask: u8 = !(((1u16 << CELL_W) - 1) as u8);
    for entry in PANEL_GLYPHS.iter() {
        let g = entry.glyph;
        assert!(g.advance > 0, "glyph {:?} has zero advance", g.ch);
        assert_eq!(g.rows.len(), CELL_H);
        for row in g.rows {
            assert_eq!(
                row & unused_mask,
                0,
                "glyph {:?} sets pixels outside the cell width",
                g.ch
            );
        }
    }
}

#[test]
fn content_hash_matches_recorded() {
    assert_eq!(PANEL_GLYPHS.content_hash(), RECORDED_HASH);
    assert_eq!(PANEL_GLYPHS.recorded_hash(), RECORDED_HASH);
}

fn to_hex(bytes: &[u8; 32]) -> std::string::String {
    use std::fmt::Write as _;
    let mut s = std::string::String::new();
    for b in bytes {
        write!(s, "{b:02x}").expect("write to String");
    }
    s
}

#[test]
fn recorded_hash_is_pinned() {
    // Pinning the exact value makes any unreviewed glyph, geometry, or
    // ordering change fail here; the same value is recorded in
    // docs/instruments/glyph-pack.md.
    assert_eq!(
        to_hex(&RECORDED_HASH),
        "281eef6229feee417c7090d8c8ea79489c017cd1c02fc7234876b2a64a532158"
    );
}

#[test]
fn canonical_form_is_reproducible() {
    let mut first = [0u8; CANONICAL_LEN];
    let mut second = [0u8; CANONICAL_LEN];
    let n1 = PANEL_GLYPHS
        .write_canonical(&mut first)
        .expect("canonical fits");
    let n2 = PANEL_GLYPHS
        .write_canonical(&mut second)
        .expect("canonical fits");
    assert_eq!(n1, CANONICAL_LEN);
    assert_eq!(n2, CANONICAL_LEN);
    assert_eq!(first, second);
    assert_eq!(PANEL_GLYPHS.content_hash(), PANEL_GLYPHS.content_hash());
}

#[test]
fn write_canonical_rejects_small_buffer() {
    let mut tiny = [0u8; 4];
    assert_eq!(
        PANEL_GLYPHS.write_canonical(&mut tiny),
        Err(GlyphError::BufferTooSmall {
            needed: CANONICAL_LEN
        })
    );
}

// A single flipped pixel bit in the `A` glyph, keeping the shipped glyph
// count so the canonical layout size is unchanged.
const CORRUPT_UPPER: [Glyph; 26] = {
    let mut arr = crate::font::UPPER_ARR;
    arr[0].rows[0] ^= 1;
    arr
};

const CORRUPT_GROUPS: &[&[Glyph]] = &[
    &crate::font::SYMBOLS_ARR,
    &crate::font::DIGITS_ARR,
    &CORRUPT_UPPER,
    &crate::font::LOWER_ARR,
];

#[test]
fn corrupt_glyph_data_fails_hash_check() {
    let corrupt = GlyphManifest::from_groups(CORRUPT_GROUPS);
    assert_ne!(corrupt.content_hash(), RECORDED_HASH);
    match corrupt.verify() {
        Err(GlyphError::ContentHashMismatch { computed, expected }) => {
            assert_ne!(computed, expected);
            assert_eq!(expected, RECORDED_HASH);
        }
        other => panic!("expected ContentHashMismatch, got {other:?}"),
    }
}

#[test]
fn flipping_one_canonical_byte_changes_the_hash() {
    let mut bytes = [0u8; CANONICAL_LEN];
    PANEL_GLYPHS
        .write_canonical(&mut bytes)
        .expect("canonical fits");
    let baseline = crate::sha256::sha256(&bytes);
    assert_eq!(baseline, RECORDED_HASH);
    bytes[HEADER_INDEX_TO_FLIP] ^= 0x01;
    assert_ne!(crate::sha256::sha256(&bytes), RECORDED_HASH);
}

const HEADER_INDEX_TO_FLIP: usize = 8;

#[test]
fn pixel_reads_match_bitmap() {
    let one = PANEL_GLYPHS.glyph('1').expect("digit one present").glyph;
    // Row 0 of `1` is `0b00100`: only the middle column is set.
    assert!(one.pixel(2, 0));
    assert!(!one.pixel(0, 0));
    assert!(!one.pixel(4, 0));
    // Out-of-cell coordinates read as unset.
    assert!(!one.pixel(CELL_W, 0));
    assert!(!one.pixel(0, CELL_H));
}
