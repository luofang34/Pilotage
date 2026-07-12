//! Stroking as a union of capsules (round joins and caps).
//!
//! Stroke semantics: a stroke is centered on its path, half its width to
//! each side. A pixel is painted when its center lies within `width / 2` of
//! the path, treating the path as the union of round-ended capsules over its
//! segments. Round joins and round caps therefore fall out of the distance
//! test rather than needing a separate construction, and each pixel is
//! tested once so overlapping segments never composite a pixel twice. Width
//! is in device pixels (equal to logical width under this rigid transform).

use crate::fixed::Fx;
use crate::surface::{PixelRect, Surface};

/// Strokes the path through `verts` (closed when `closed`) at `width`.
pub(crate) fn stroke_path(
    surface: &mut Surface<'_>,
    clip: PixelRect,
    verts: &[[Fx; 2]],
    closed: bool,
    width: f32,
    color: [u8; 4],
) {
    if width <= 0.0 || verts.is_empty() || clip.is_empty() {
        return;
    }
    let hw = width / 2.0;
    let region = bounds(verts, hw).intersect(clip);
    if region.is_empty() {
        return;
    }
    let seg_count = if verts.len() == 1 {
        0
    } else if closed {
        verts.len()
    } else {
        verts.len() - 1
    };
    for py in region.top..region.bottom {
        let cy = py as f32 + 0.5;
        for px in region.left..region.right {
            let cx = px as f32 + 0.5;
            if covered(verts, seg_count, cx, cy, hw) {
                surface.composite(px, py, color);
            }
        }
    }
}

/// Whether the pixel center is within `hw` of any segment (or the single
/// vertex, for a one-point path).
fn covered(verts: &[[Fx; 2]], seg_count: usize, cx: f32, cy: f32, hw: f32) -> bool {
    if seg_count == 0 {
        return point_dist(verts[0], cx, cy) <= hw;
    }
    let n = verts.len();
    for i in 0..seg_count {
        if seg_dist(verts[i], verts[(i + 1) % n], cx, cy) <= hw {
            return true;
        }
    }
    false
}

fn point_dist(p: [Fx; 2], cx: f32, cy: f32) -> f32 {
    let dx = cx - p[0].to_f32();
    let dy = cy - p[1].to_f32();
    libm::sqrtf(dx * dx + dy * dy)
}

/// Distance from `(cx, cy)` to the segment `a`–`b`.
fn seg_dist(a: [Fx; 2], b: [Fx; 2], cx: f32, cy: f32) -> f32 {
    let (ax, ay) = (a[0].to_f32(), a[1].to_f32());
    let ex = b[0].to_f32() - ax;
    let ey = b[1].to_f32() - ay;
    let len2 = ex * ex + ey * ey;
    let t = if len2 <= 0.0 {
        0.0
    } else {
        (((cx - ax) * ex + (cy - ay) * ey) / len2).clamp(0.0, 1.0)
    };
    let qx = ax + t * ex;
    let qy = ay + t * ey;
    libm::sqrtf((cx - qx) * (cx - qx) + (cy - qy) * (cy - qy))
}

/// Pixel bounding box of the path expanded by the half-width.
fn bounds(verts: &[[Fx; 2]], hw: f32) -> PixelRect {
    let mut min_x = verts[0][0].to_f32();
    let mut max_x = min_x;
    let mut min_y = verts[0][1].to_f32();
    let mut max_y = min_y;
    for v in &verts[1..] {
        min_x = min_x.min(v[0].to_f32());
        max_x = max_x.max(v[0].to_f32());
        min_y = min_y.min(v[1].to_f32());
        max_y = max_y.max(v[1].to_f32());
    }
    PixelRect {
        left: libm::floorf(min_x - hw) as i32,
        top: libm::floorf(min_y - hw) as i32,
        right: libm::ceilf(max_x + hw) as i32 + 1,
        bottom: libm::ceilf(max_y + hw) as i32 + 1,
    }
}

#[cfg(test)]
mod tests;
