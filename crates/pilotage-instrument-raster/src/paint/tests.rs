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

fn pix(buf: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * w + x) * 4) as usize;
    [buf[at], buf[at + 1], buf[at + 2], buf[at + 3]]
}

#[test]
fn axis_aligned_rectangle_fills_pixel_centers_inside() {
    let quad = verts(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]);
    let buf = painted(8, 8, |s| fill_polygon(s, s.bounds(), &quad, RED));
    assert_eq!(pix(&buf, 8, 0, 0), RED);
    assert_eq!(pix(&buf, 8, 3, 3), RED);
    // Centers at 4.5 fall outside the half-open [0,4) span.
    assert_eq!(pix(&buf, 8, 4, 0), [0, 0, 0, 0]);
    assert_eq!(pix(&buf, 8, 0, 4), [0, 0, 0, 0]);
}

#[test]
fn triangle_fills_its_interior_only() {
    let tri = verts(&[(1.0, 1.0), (7.0, 1.0), (1.0, 7.0)]);
    let buf = painted(8, 8, |s| fill_polygon(s, s.bounds(), &tri, RED));
    assert_eq!(pix(&buf, 8, 2, 2), RED);
    // The opposite corner is outside the hypotenuse.
    assert_eq!(pix(&buf, 8, 6, 6), [0, 0, 0, 0]);
}

#[test]
fn non_zero_rule_fills_a_self_intersecting_star_center() {
    // A pentagram traced by connecting every second point winds the center
    // twice: non-zero fills it, even-odd would leave it hollow.
    let star = verts(&[
        (10.0, 2.0),
        (14.70, 16.47),
        (2.39, 7.53),
        (17.61, 7.53),
        (5.30, 16.47),
    ]);
    let buf = painted(20, 20, |s| fill_polygon(s, s.bounds(), &star, RED));
    assert_eq!(
        pix(&buf, 20, 10, 10),
        RED,
        "star center is filled (non-zero)"
    );
    assert_eq!(pix(&buf, 20, 0, 0), [0, 0, 0, 0], "outside stays empty");
}

#[test]
fn clip_confines_the_fill() {
    let quad = verts(&[(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0)]);
    let clip = PixelRect {
        left: 2,
        top: 2,
        right: 5,
        bottom: 5,
    };
    let buf = painted(8, 8, |s| fill_polygon(s, clip, &quad, RED));
    assert_eq!(pix(&buf, 8, 3, 3), RED);
    assert_eq!(pix(&buf, 8, 1, 1), [0, 0, 0, 0]);
    assert_eq!(pix(&buf, 8, 5, 5), [0, 0, 0, 0]);
}

#[test]
fn degenerate_polygons_paint_nothing() {
    let line = verts(&[(0.0, 0.0), (8.0, 8.0)]);
    let buf = painted(8, 8, |s| fill_polygon(s, s.bounds(), &line, RED));
    assert!(buf.iter().all(|&b| b == 0));
}
