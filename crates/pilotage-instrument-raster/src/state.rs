//! The graphics-state stack: transform, clip, and paints under save/restore.

use pilotage_instrument_scene::{MAX_STACK_DEPTH, Rgba8};

use crate::error::RasterError;
use crate::fixed::{FRAC_BITS, Fx, HALF};
use crate::surface::PixelRect;
use crate::transform::Affine;

/// One saved graphics state: transform, clip, and the active paints.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GraphicsState {
    /// Current transform matrix.
    pub(crate) ctm: Affine,
    /// Current clip rectangle in device pixels.
    pub(crate) clip: PixelRect,
    /// Fill color as straight-alpha RGBA8.
    pub(crate) fill: [u8; 4],
    /// Stroke color as straight-alpha RGBA8.
    pub(crate) stroke_color: [u8; 4],
    /// Stroke width in device pixels (never negative).
    pub(crate) stroke_width: f32,
}

fn rgba(color: Rgba8) -> [u8; 4] {
    [color.r, color.g, color.b, color.a]
}

/// Rounds a Q8.8 device coordinate to the nearest pixel boundary, matching
/// the pixel-center coverage rule so clips align with fills.
fn round_px(raw: i64) -> i32 {
    ((raw + HALF as i64) >> FRAC_BITS) as i32
}

/// The transform/clip/paint state and its bounded save stack.
pub(crate) struct RenderState {
    current: GraphicsState,
    stack: [GraphicsState; MAX_STACK_DEPTH],
    depth: usize,
}

impl RenderState {
    /// Starts at identity transform, the surface-bounds clip, and the
    /// Canvas defaults (opaque black paints, unit stroke width).
    pub(crate) fn new(clip: PixelRect) -> Self {
        let current = GraphicsState {
            ctm: Affine::IDENTITY,
            clip,
            fill: [0, 0, 0, 255],
            stroke_color: [0, 0, 0, 255],
            stroke_width: 1.0,
        };
        Self {
            current,
            stack: [current; MAX_STACK_DEPTH],
            depth: 0,
        }
    }

    /// The active graphics state.
    pub(crate) fn current(&self) -> &GraphicsState {
        &self.current
    }

    /// The active transform, mutable for translate/rotate composition.
    pub(crate) fn ctm_mut(&mut self) -> &mut Affine {
        &mut self.current.ctm
    }

    /// Sets the fill paint.
    pub(crate) fn set_fill(&mut self, color: Rgba8) {
        self.current.fill = rgba(color);
    }

    /// Sets the stroke paint; a negative width clamps to zero (paints
    /// nothing) rather than inverting the offset.
    pub(crate) fn set_stroke(&mut self, color: Rgba8, width: f32) -> Result<(), RasterError> {
        if !width.is_finite() {
            return Err(RasterError::NonFinite);
        }
        self.current.stroke_color = rgba(color);
        self.current.stroke_width = width.max(0.0);
        Ok(())
    }

    /// Pushes a copy of the active state.
    pub(crate) fn save(&mut self) -> Result<(), RasterError> {
        let slot = self
            .stack
            .get_mut(self.depth)
            .ok_or(RasterError::StackOverflow {
                limit: MAX_STACK_DEPTH,
            })?;
        *slot = self.current;
        self.depth = self.depth.wrapping_add(1);
        Ok(())
    }

    /// Pops to the most recently saved state.
    pub(crate) fn restore(&mut self) -> Result<(), RasterError> {
        let depth = self
            .depth
            .checked_sub(1)
            .ok_or(RasterError::UnbalancedRestore)?;
        self.current = *self
            .stack
            .get(depth)
            .ok_or(RasterError::UnbalancedRestore)?;
        self.depth = depth;
        Ok(())
    }

    /// Intersects the clip with the device-space bounding box of a logical
    /// rectangle's transformed corners (rect-intersection clipping only;
    /// under rotation this is the conservative axis-aligned bound).
    pub(crate) fn clip_rect(&mut self, corners: &[[Fx; 2]; 4]) {
        let mut min_x = corners[0][0].raw();
        let mut max_x = min_x;
        let mut min_y = corners[0][1].raw();
        let mut max_y = min_y;
        for corner in &corners[1..] {
            min_x = min_x.min(corner[0].raw());
            max_x = max_x.max(corner[0].raw());
            min_y = min_y.min(corner[1].raw());
            max_y = max_y.max(corner[1].raw());
        }
        let rect = PixelRect {
            left: round_px(min_x),
            top: round_px(min_y),
            right: round_px(max_x),
            bottom: round_px(max_y),
        };
        self.current.clip = self.current.clip.intersect(rect);
    }
}

#[cfg(test)]
mod tests;
