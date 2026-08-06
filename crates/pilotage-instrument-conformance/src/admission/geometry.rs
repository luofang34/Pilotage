//! Design-space geometry for the admission checks: the same affine
//! semantics the reference rasterizer applies (post-multiplied
//! translate/rotate, y-down clockwise rotation), with rotated shapes
//! reduced to conservative axis-aligned bounds.

use pilotage_instrument_scene::{HAlign, VAlign, nominal_text_ink_width};

/// An axis-aligned rectangle in design-frame units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Rect {
    pub(super) min_x: f32,
    pub(super) min_y: f32,
    pub(super) max_x: f32,
    pub(super) max_y: f32,
}

impl Rect {
    pub(super) fn intersects(&self, other: &Rect) -> bool {
        self.min_x < other.max_x
            && other.min_x < self.max_x
            && self.min_y < other.max_y
            && other.min_y < self.max_y
    }

    pub(super) fn contains(&self, other: &Rect) -> bool {
        other.min_x >= self.min_x
            && other.max_x <= self.max_x
            && other.min_y >= self.min_y
            && other.max_y <= self.max_y
    }

    pub(super) fn intersect(&self, other: &Rect) -> Rect {
        Rect {
            min_x: self.min_x.max(other.min_x),
            min_y: self.min_y.max(other.min_y),
            max_x: self.max_x.min(other.max_x),
            max_y: self.max_y.min(other.max_y),
        }
    }
}

/// Row-major affine transform, matching the rasterizer's `Affine`:
/// `x' = a·x + c·y + e`, `y' = b·x + d·y + f`.
#[derive(Debug, Clone, Copy)]
pub(super) struct Ctm {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl Ctm {
    pub(super) const IDENTITY: Ctm = Ctm {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// Post-multiplies a translation, as the IR's `Translate` does.
    pub(super) fn translate(&mut self, tx: f32, ty: f32) {
        self.e += self.a * tx + self.c * ty;
        self.f += self.b * tx + self.d * ty;
    }

    /// Post-multiplies a rotation; positive is clockwise in the y-down
    /// logical space, matching the IR.
    pub(super) fn rotate(&mut self, radians: f32) {
        let (s, k) = (radians.sin(), radians.cos());
        let (a, b, c, d) = (self.a, self.b, self.c, self.d);
        self.a = a * k + c * s;
        self.b = b * k + d * s;
        self.c = -a * s + c * k;
        self.d = -b * s + d * k;
    }

    /// Whether the transform is a pure translation/scale (no rotation
    /// or shear): the case where a mapped rectangle's bounds ARE the
    /// rectangle, so bbox containment is exact rather than merely
    /// conservative.
    pub(super) fn is_axis_aligned(&self) -> bool {
        self.b == 0.0 && self.c == 0.0
    }

    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    /// The axis-aligned bounds of `rect`'s four mapped corners —
    /// conservative under rotation.
    pub(super) fn map_rect(&self, rect: &Rect) -> Rect {
        let corners = [
            self.apply(rect.min_x, rect.min_y),
            self.apply(rect.max_x, rect.min_y),
            self.apply(rect.min_x, rect.max_y),
            self.apply(rect.max_x, rect.max_y),
        ];
        let mut out = Rect {
            min_x: f32::INFINITY,
            min_y: f32::INFINITY,
            max_x: f32::NEG_INFINITY,
            max_y: f32::NEG_INFINITY,
        };
        for (x, y) in corners {
            out.min_x = out.min_x.min(x);
            out.min_y = out.min_y.min(y);
            out.max_x = out.max_x.max(x);
            out.max_y = out.max_y.max(y);
        }
        out
    }
}

/// The local-space ink rectangle of a text run: nominal ink width from
/// the scene text-metrics contract, cap height approximated by the run
/// size, positioned by the anchor.
pub(super) fn text_rect(x: f32, y: f32, size: f32, h: HAlign, v: VAlign, chars: usize) -> Rect {
    let width = nominal_text_ink_width(size, chars);
    let min_x = match h {
        HAlign::Left => x,
        HAlign::Center => x - width / 2.0,
        HAlign::Right => x - width,
    };
    let min_y = match v {
        VAlign::Baseline | VAlign::Bottom => y - size,
        VAlign::Middle => y - size / 2.0,
        VAlign::Top => y,
    };
    Rect {
        min_x,
        min_y,
        max_x: min_x + width,
        max_y: min_y + size,
    }
}
