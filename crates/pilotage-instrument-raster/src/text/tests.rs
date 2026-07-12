#![allow(clippy::expect_used, clippy::panic)]

use std::vec::Vec;

use pilotage_instrument_scene::{Anchor, HAlign, VAlign};

use super::*;
use crate::report::FramebufferDims;
use crate::surface::Surface;
use crate::transform::Affine;

const WHITE: [u8; 4] = [255, 255, 255, 255];

fn painted(run: Run) -> Vec<u8> {
    let mut buf = std::vec![0u8; 40 * 40 * 4];
    {
        let mut s = Surface::new(&mut buf, FramebufferDims::tight(40, 40)).expect("surface");
        let clip = s.bounds();
        draw_placeholder(&mut s, clip, &Affine::IDENTITY, run, WHITE).expect("finite run");
    }
    buf
}

fn covered(buf: &[u8], x: u32, y: u32) -> bool {
    buf[((y * 40 + x) * 4 + 3) as usize] != 0
}

fn run(x: f32, y: f32, size: f32, anchor: Anchor, byte_len: usize) -> Run {
    Run {
        x,
        y,
        size,
        anchor,
        byte_len,
    }
}

#[test]
fn empty_run_paints_nothing() {
    let buf = painted(run(5.0, 20.0, 10.0, Anchor::BASELINE_LEFT, 0));
    assert!(buf.iter().all(|&b| b == 0));
}

#[test]
fn non_positive_size_paints_nothing() {
    let buf = painted(run(5.0, 20.0, 0.0, Anchor::BASELINE_LEFT, 3));
    assert!(buf.iter().all(|&b| b == 0));
}

#[test]
fn placeholder_draws_a_hollow_box() {
    let buf = painted(run(5.0, 20.0, 10.0, Anchor::BASELINE_LEFT, 3));
    // Left edge of the box is painted; its interior is hollow.
    assert!(covered(&buf, 5, 17), "left edge painted");
    assert!(!covered(&buf, 14, 17), "interior hollow");
}

#[test]
fn horizontal_anchor_shifts_the_box() {
    let top = Anchor {
        h: HAlign::Left,
        v: VAlign::Top,
    };
    let centered = Anchor {
        h: HAlign::Center,
        v: VAlign::Top,
    };
    let left = painted(run(20.0, 5.0, 10.0, top, 3));
    let center = painted(run(20.0, 5.0, 10.0, centered, 3));
    assert_ne!(left, center, "anchor changes the box position");
}

#[test]
fn placeholder_is_deterministic() {
    let a = painted(run(5.0, 20.0, 10.0, Anchor::CENTER, 4));
    let b = painted(run(5.0, 20.0, 10.0, Anchor::CENTER, 4));
    assert_eq!(a, b);
}
