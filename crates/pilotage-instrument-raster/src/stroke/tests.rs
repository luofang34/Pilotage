#![allow(clippy::expect_used, clippy::panic)]

use std::vec::Vec;

use super::*;
use crate::report::FramebufferDims;
use crate::surface::Surface;

const RED: [u8; 4] = [255, 0, 0, 255];

fn verts(coords: &[(f32, f32)]) -> Vec<[Fx; 2]> {
    coords
        .iter()
        .map(|&(x, y)| [Fx::snap(x).expect("finite"), Fx::snap(y).expect("finite")])
        .collect()
}

fn painted(w: u32, h: u32, draw: impl FnOnce(&mut Surface<'_>)) -> Vec<u8> {
    let mut buf = std::vec![0u8; (w * h * 4) as usize];
    {
        let mut s = Surface::new(&mut buf, FramebufferDims::tight(w, h)).expect("surface");
        draw(&mut s);
    }
    buf
}

fn covered_px(buf: &[u8], w: u32, x: u32, y: u32) -> bool {
    let at = ((y * w + x) * 4) as usize;
    buf[at + 3] != 0
}

#[test]
fn horizontal_line_has_the_requested_width() {
    let line = verts(&[(2.0, 4.0), (6.0, 4.0)]);
    let buf = painted(8, 8, |s| stroke_path(s, s.bounds(), &line, false, 2.0, RED));
    // A width-2 centered stroke covers the two rows within half a pixel.
    assert!(covered_px(&buf, 8, 4, 3));
    assert!(covered_px(&buf, 8, 4, 4));
    assert!(!covered_px(&buf, 8, 4, 2));
    assert!(!covered_px(&buf, 8, 4, 5));
}

#[test]
fn round_caps_extend_past_the_endpoints() {
    let line = verts(&[(2.0, 4.0), (6.0, 4.0)]);
    let buf = painted(8, 8, |s| stroke_path(s, s.bounds(), &line, false, 2.0, RED));
    assert!(covered_px(&buf, 8, 6, 4), "cap reaches just past the end");
    assert!(!covered_px(&buf, 8, 7, 4), "but not a full pixel beyond");
}

#[test]
fn zero_width_paints_nothing() {
    let line = verts(&[(0.0, 0.0), (8.0, 8.0)]);
    let buf = painted(8, 8, |s| stroke_path(s, s.bounds(), &line, false, 0.0, RED));
    assert!(buf.iter().all(|&b| b == 0));
}

#[test]
fn single_point_path_draws_a_round_dot() {
    let dot = verts(&[(4.0, 4.0)]);
    let buf = painted(8, 8, |s| stroke_path(s, s.bounds(), &dot, false, 3.0, RED));
    assert!(covered_px(&buf, 8, 4, 4));
    assert!(!covered_px(&buf, 8, 0, 0));
}

#[test]
fn closed_path_strokes_the_closing_edge() {
    // The open path draws the top and hypotenuse edges; the closing edge is
    // the left vertical from (1,6) back to (1,1).
    let tri = verts(&[(1.0, 1.0), (6.0, 1.0), (1.0, 6.0)]);
    let open = painted(8, 8, |s| stroke_path(s, s.bounds(), &tri, false, 1.0, RED));
    let closed = painted(8, 8, |s| stroke_path(s, s.bounds(), &tri, true, 1.0, RED));
    // A point on the left edge is covered only once the path is closed.
    assert!(!covered_px(&open, 8, 1, 3));
    assert!(covered_px(&closed, 8, 1, 3));
}
