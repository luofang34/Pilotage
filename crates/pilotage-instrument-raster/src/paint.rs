//! Polygon and rectangle fills by the non-zero winding rule.
//!
//! Fill rule: a pixel belongs to a polygon when the winding number of the
//! polygon's edges around the pixel center is non-zero. Coverage is sampled
//! once, at the pixel center `(px + 0.5, py + 0.5)`, with no anti-aliasing,
//! so a pixel is wholly inside or wholly outside and the frame is exactly
//! reproducible. Winding is computed in Q8.8 integer arithmetic widened to
//! `i64`, independent of floating point.

use crate::fixed::{FRAC_BITS, Fx};
use crate::surface::{PixelRect, Surface};

/// Fills the closed polygon through `verts` with `color`, clipped to `clip`.
pub(crate) fn fill_polygon(
    surface: &mut Surface<'_>,
    clip: PixelRect,
    verts: &[[Fx; 2]],
    color: [u8; 4],
) {
    if verts.len() < 3 || clip.is_empty() {
        return;
    }
    let region = bounds(verts).intersect(clip);
    if region.is_empty() {
        return;
    }
    for py in region.top..region.bottom {
        let cy = Fx::pixel_center(py).raw();
        for px in region.left..region.right {
            surface.count_sample();
            // The winding test always walks every edge, so the priced count
            // is exact, not an upper bound.
            surface.count_polygon_edge_tests(verts.len() as u64);
            let cx = Fx::pixel_center(px).raw();
            if winding_nonzero(verts, cx, cy) {
                surface.composite(px, py, color);
            }
        }
    }
}

/// The half-open pixel bounding box covering every pixel whose center could
/// fall inside the vertices.
fn bounds(verts: &[[Fx; 2]]) -> PixelRect {
    let mut min_x = verts[0][0].raw();
    let mut max_x = min_x;
    let mut min_y = verts[0][1].raw();
    let mut max_y = min_y;
    for v in &verts[1..] {
        min_x = min_x.min(v[0].raw());
        max_x = max_x.max(v[0].raw());
        min_y = min_y.min(v[1].raw());
        max_y = max_y.max(v[1].raw());
    }
    PixelRect {
        left: (min_x >> FRAC_BITS) as i32,
        top: (min_y >> FRAC_BITS) as i32,
        right: ((max_x >> FRAC_BITS) + 1) as i32,
        bottom: ((max_y >> FRAC_BITS) + 1) as i32,
    }
}

/// Non-zero winding test for a pixel center against the polygon edges.
fn winding_nonzero(verts: &[[Fx; 2]], px: i64, py: i64) -> bool {
    let n = verts.len();
    let mut wn: i32 = 0;
    for i in 0..n {
        let a = verts[i];
        let b = verts[(i + 1) % n];
        let ay = a[1].raw();
        let by = b[1].raw();
        if ay <= py {
            if by > py && is_left(a, b, px, py) > 0 {
                wn = wn.wrapping_add(1);
            }
        } else if by <= py && is_left(a, b, px, py) < 0 {
            wn = wn.wrapping_sub(1);
        }
    }
    wn != 0
}

/// Signed area of the triangle (a, b, p); positive when p is left of a→b.
fn is_left(a: [Fx; 2], b: [Fx; 2], px: i64, py: i64) -> i64 {
    (b[0].raw() - a[0].raw()) * (py - a[1].raw()) - (px - a[0].raw()) * (b[1].raw() - a[1].raw())
}

#[cfg(test)]
mod tests;
