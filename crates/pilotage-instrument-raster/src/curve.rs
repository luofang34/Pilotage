//! Circle and arc coverage by exact distance to the center.
//!
//! A circle is rotation-invariant, so under this rigid transform its
//! transformed center plus unchanged radius reproduce it exactly. Coverage
//! samples the pixel center once with no anti-aliasing: a fill covers pixels
//! within the radius; a circle or arc stroke covers pixels within the
//! half-width of the ideal ring. An arc additionally tests angular
//! membership and paints round caps at its two endpoints, matching the
//! stroke module's round-cap semantics.

use core::f32::consts::TAU;

use crate::surface::{PixelRect, Surface};

/// A device-space center and radius after transform and snapping.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Disc {
    /// Center x in device pixels.
    pub(crate) cx: f32,
    /// Center y in device pixels.
    pub(crate) cy: f32,
    /// Radius in device pixels (never negative).
    pub(crate) r: f32,
}

/// Fills the disc with `color`.
pub(crate) fn fill_circle(surface: &mut Surface<'_>, clip: PixelRect, disc: Disc, color: [u8; 4]) {
    paint(surface, clip, disc, disc.r, color, 0, |dist, _, _| {
        dist <= disc.r
    });
}

/// Strokes the circle outline centered on the ideal ring at half-width `hw`.
pub(crate) fn stroke_circle(
    surface: &mut Surface<'_>,
    clip: PixelRect,
    disc: Disc,
    hw: f32,
    color: [u8; 4],
) {
    paint(surface, clip, disc, disc.r + hw, color, 0, |dist, _, _| {
        libm::fabsf(dist - disc.r) <= hw
    });
}

/// Strokes an arc from `start` sweeping `sweep` radians (device angles),
/// with round caps at both ends.
pub(crate) fn stroke_arc(
    surface: &mut Surface<'_>,
    clip: PixelRect,
    disc: Disc,
    start: f32,
    sweep: f32,
    hw: f32,
    color: [u8; 4],
) {
    let cap0 = end_point(disc, start);
    let cap1 = end_point(disc, start + sweep);
    // Each arc sample may evaluate two cap distances, `atan2f`, and `fmodf`
    // beyond its disc test — priced as one arc test per sample.
    paint(
        surface,
        clip,
        disc,
        disc.r + hw,
        color,
        1,
        |dist, dx, dy| {
            if near(dx, dy, cap0, hw) || near(dx, dy, cap1, hw) {
                return true;
            }
            libm::fabsf(dist - disc.r) <= hw && arc_contains(libm::atan2f(dy, dx), start, sweep)
        },
    );
}

/// Iterates the pixel box of radius `reach` around the center, compositing
/// where `covered` accepts the pixel. `covered` receives the center distance
/// and the offset `(dx, dy)` from the center.
fn paint(
    surface: &mut Surface<'_>,
    clip: PixelRect,
    disc: Disc,
    reach: f32,
    color: [u8; 4],
    arc_tests_per_sample: u64,
    covered: impl Fn(f32, f32, f32) -> bool,
) {
    if reach < 0.0 || clip.is_empty() {
        return;
    }
    let region = bounds(disc.cx, disc.cy, reach).intersect(clip);
    for py in region.top..region.bottom {
        let dy = (py as f32 + 0.5) - disc.cy;
        for px in region.left..region.right {
            surface.count_sample();
            surface.count_disc_tests(1);
            surface.count_arc_tests(arc_tests_per_sample);
            let dx = (px as f32 + 0.5) - disc.cx;
            let dist = libm::sqrtf(dx * dx + dy * dy);
            if covered(dist, dx, dy) {
                surface.composite(px, py, color);
            }
        }
    }
}

fn end_point(disc: Disc, angle: f32) -> (f32, f32) {
    (disc.r * libm::cosf(angle), disc.r * libm::sinf(angle))
}

fn near(dx: f32, dy: f32, cap: (f32, f32), hw: f32) -> bool {
    let ex = dx - cap.0;
    let ey = dy - cap.1;
    libm::sqrtf(ex * ex + ey * ey) <= hw
}

/// Whether device angle `ang` lies on the arc `[start, start + sweep]`,
/// handling either sweep sign and a full turn.
fn arc_contains(ang: f32, start: f32, sweep: f32) -> bool {
    if libm::fabsf(sweep) >= TAU {
        return true;
    }
    let mut d = libm::fmodf(ang - start, TAU);
    if d < 0.0 {
        d += TAU;
    }
    if sweep >= 0.0 {
        d <= sweep
    } else {
        d >= TAU + sweep
    }
}

fn bounds(cx: f32, cy: f32, reach: f32) -> PixelRect {
    PixelRect {
        left: libm::floorf(cx - reach) as i32,
        top: libm::floorf(cy - reach) as i32,
        right: libm::ceilf(cx + reach) as i32 + 1,
        bottom: libm::ceilf(cy + reach) as i32 + 1,
    }
}

#[cfg(test)]
mod tests;
