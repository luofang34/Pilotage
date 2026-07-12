//! The current transform matrix and its Canvas2D-order composition.
//!
//! An [`Affine`] maps a logical point to device pixels. The initial
//! transform is the identity: one logical unit is one device pixel, top-left
//! origin. [`Cmd::Translate`](pilotage_instrument_scene::Cmd::Translate) and
//! [`Cmd::Rotate`](pilotage_instrument_scene::Cmd::Rotate) post-multiply, so
//! they act in the current local frame exactly as `CanvasRenderingContext2D`
//! `translate`/`rotate` do, composing in command order. The IR has no scale
//! operation and the initial transform is unit-scale, so this family stays a
//! rigid motion: a stroke width in logical units equals its device pixel
//! width, and a circle stays a circle.

use crate::error::RasterError;
use crate::fixed::Fx;

/// A 2x3 affine map `x' = a*x + c*y + e`, `y' = b*x + d*y + f`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Affine {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl Affine {
    /// The identity map (logical units are device pixels).
    pub(crate) const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// Post-multiplies a translation, as `ctx.translate(tx, ty)` does.
    pub(crate) fn translate(&mut self, tx: f32, ty: f32) {
        self.e += self.a * tx + self.c * ty;
        self.f += self.b * tx + self.d * ty;
    }

    /// Post-multiplies a rotation; positive is clockwise in the y-down
    /// device space, matching the IR.
    pub(crate) fn rotate(&mut self, radians: f32) {
        let (s, k) = (libm::sinf(radians), libm::cosf(radians));
        let (a, b, c, d) = (self.a, self.b, self.c, self.d);
        self.a = a * k + c * s;
        self.b = b * k + d * s;
        self.c = -a * s + c * k;
        self.d = -b * s + d * k;
    }

    /// Maps a logical point to a snapped device coordinate (the single
    /// quantization point), or fails on non-finite / out-of-range results.
    pub(crate) fn map(&self, x: f32, y: f32) -> Result<[Fx; 2], RasterError> {
        let dx = self.a * x + self.c * y + self.e;
        let dy = self.b * x + self.d * y + self.f;
        Ok([Fx::snap(dx)?, Fx::snap(dy)?])
    }

    /// The accumulated rotation angle, used to place an arc's angular span
    /// in device space. Exact for this rigid-motion family.
    pub(crate) fn rotation(&self) -> f32 {
        libm::atan2f(self.b, self.a)
    }
}
