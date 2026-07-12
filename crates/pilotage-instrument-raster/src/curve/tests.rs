#![allow(clippy::expect_used, clippy::panic)]

use core::f32::consts::FRAC_PI_2;
use std::vec::Vec;

use super::*;
use crate::report::FramebufferDims;
use crate::surface::Surface;

const RED: [u8; 4] = [255, 0, 0, 255];

fn disc() -> Disc {
    Disc {
        cx: 10.0,
        cy: 10.0,
        r: 5.0,
    }
}

fn painted(draw: impl FnOnce(&mut Surface<'_>)) -> Vec<u8> {
    let mut buf = std::vec![0u8; 20 * 20 * 4];
    {
        let mut s = Surface::new(&mut buf, FramebufferDims::tight(20, 20)).expect("surface");
        draw(&mut s);
    }
    buf
}

fn covered(buf: &[u8], x: u32, y: u32) -> bool {
    buf[((y * 20 + x) * 4 + 3) as usize] != 0
}

#[test]
fn filled_circle_covers_the_interior_not_beyond_the_radius() {
    let buf = painted(|s| fill_circle(s, s.bounds(), disc(), RED));
    assert!(covered(&buf, 10, 10));
    assert!(covered(&buf, 10, 14));
    assert!(!covered(&buf, 10, 16));
}

#[test]
fn stroked_circle_is_hollow() {
    let buf = painted(|s| stroke_circle(s, s.bounds(), disc(), 1.0, RED));
    assert!(!covered(&buf, 10, 10), "center is empty");
    assert!(covered(&buf, 15, 10), "ring at the radius is painted");
}

#[test]
fn arc_paints_only_its_angular_span() {
    // Quarter arc from +x sweeping clockwise to +y.
    let buf = painted(|s| stroke_arc(s, s.bounds(), disc(), 0.0, FRAC_PI_2, 1.0, RED));
    assert!(covered(&buf, 15, 10), "east end is on the arc");
    assert!(!covered(&buf, 5, 10), "west is outside the sweep");
}

#[test]
fn full_turn_sweep_paints_the_whole_ring() {
    let buf = painted(|s| stroke_arc(s, s.bounds(), disc(), 0.0, 7.0, 1.0, RED));
    assert!(covered(&buf, 15, 10));
    assert!(covered(&buf, 5, 10));
}

#[test]
fn negative_reach_paints_nothing() {
    let empty = Disc {
        cx: 10.0,
        cy: 10.0,
        r: 0.0,
    };
    let buf = painted(|s| stroke_circle(s, s.bounds(), empty, -1.0, RED));
    assert!(buf.iter().all(|&b| b == 0));
}
