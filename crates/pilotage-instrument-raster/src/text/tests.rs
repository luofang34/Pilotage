#![allow(clippy::expect_used, clippy::panic)]

use std::vec::Vec;

use pilotage_instrument_glyphs::GlyphError;
use pilotage_instrument_scene::{Anchor, HAlign, VAlign};

use super::*;
use crate::error::RasterError;
use crate::report::FramebufferDims;
use crate::surface::Surface;
use crate::transform::Affine;

const WHITE: [u8; 4] = [255, 255, 255, 255];

fn painted(run: Run<'_>) -> Result<Vec<u8>, RasterError> {
    let mut buf = std::vec![0u8; 64 * 64 * 4];
    {
        let mut s = Surface::new(&mut buf, FramebufferDims::tight(64, 64)).expect("surface");
        let clip = s.bounds();
        draw_run(&mut s, clip, &Affine::IDENTITY, run, WHITE)?;
    }
    Ok(buf)
}

fn covered(buf: &[u8], x: u32, y: u32) -> bool {
    buf[((y * 64 + x) * 4 + 3) as usize] != 0
}

fn run(x: f32, y: f32, size: f32, anchor: Anchor, text: &str) -> Run<'_> {
    Run {
        x,
        y,
        size,
        anchor,
        text,
    }
}

#[test]
fn empty_run_paints_nothing() {
    let buf = painted(run(5.0, 20.0, 10.0, Anchor::BASELINE_LEFT, "")).expect("empty ok");
    assert!(buf.iter().all(|&b| b == 0));
}

#[test]
fn non_positive_size_paints_nothing() {
    let buf = painted(run(5.0, 20.0, 0.0, Anchor::BASELINE_LEFT, "123")).expect("zero size ok");
    assert!(buf.iter().all(|&b| b == 0));
}

#[test]
fn glyph_pixels_paint_above_the_baseline() {
    // "1" at cell-native scale (size == cell height * 2 = 14): pixels land
    // strictly within [x, x+advance) x [y-size, y).
    let buf = painted(run(8.0, 40.0, 14.0, Anchor::BASELINE_LEFT, "1")).expect("renders");
    assert!(!buf.iter().all(|&b| b == 0), "glyph painted something");
    for y in 0..64 {
        for x in 0..64 {
            if covered(&buf, x, y) {
                assert!((8..20).contains(&x), "x {x} inside the advance box");
                assert!((26..40).contains(&y), "y {y} above the baseline");
            }
        }
    }
}

#[test]
fn uncovered_character_is_a_typed_failure_never_a_substitute() {
    let mut buf = std::vec![0u8; 64 * 64 * 4];
    let mut s = Surface::new(&mut buf, FramebufferDims::tight(64, 64)).expect("surface");
    let clip = s.bounds();
    let err = draw_run(
        &mut s,
        clip,
        &Affine::IDENTITY,
        run(5.0, 20.0, 10.0, Anchor::BASELINE_LEFT, "\u{00e9}"),
        WHITE,
    )
    .expect_err("no glyph for e-acute");
    assert!(matches!(
        err,
        RasterError::Glyph(GlyphError::MissingGlyph { .. })
    ));
}

#[test]
fn horizontal_anchor_shifts_the_run() {
    let top = Anchor {
        h: HAlign::Left,
        v: VAlign::Top,
    };
    let centered = Anchor {
        h: HAlign::Center,
        v: VAlign::Top,
    };
    let left = painted(run(30.0, 5.0, 10.0, top, "88")).expect("renders");
    let center = painted(run(30.0, 5.0, 10.0, centered, "88")).expect("renders");
    assert_ne!(left, center, "anchor changes the run position");
}

#[test]
fn glyph_text_is_deterministic() {
    let a = painted(run(5.0, 30.0, 12.0, Anchor::CENTER, "360\u{00b0}")).expect("renders");
    let b = painted(run(5.0, 30.0, 12.0, Anchor::CENTER, "360\u{00b0}")).expect("renders");
    assert_eq!(a, b);
}
