//! The single subpixel quantization boundary: Q8.8 fixed point.
//!
//! Every device-space coordinate the affine transform produces is snapped
//! to this grid, once, at the moment it is produced (see [`Fx::snap`]).
//! All later edge and distance tests read the snapped value, so a frame is
//! independent of how the platform rounds the `f32` transform arithmetic
//! beyond this one point. The unit is 1/256 of a device pixel.

use crate::error::RasterError;

/// Fixed-point fractional bits: 1 unit is `1 / 256` device pixels.
pub(crate) const FRAC_BITS: u32 = 8;

/// Fixed-point scale (`2^FRAC_BITS`).
pub(crate) const ONE: i32 = 1 << FRAC_BITS;

/// Half a pixel in fixed point; a pixel center offset from its top-left.
pub(crate) const HALF: i32 = ONE / 2;

/// Largest absolute device coordinate accepted, in pixels. A snapped
/// coordinate past this fails rather than wrapping the fixed-point range;
/// `32767 * 256` stays well inside `i32` and cross products stay inside
/// `i64`.
pub(crate) const COORD_LIMIT_PX: f32 = 32767.0;

const COORD_LIMIT_RAW: i32 = 32767 * ONE;

/// A device coordinate on the Q8.8 grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Fx(i32);

impl Fx {
    /// The grid origin, a filler for fixed-size vertex buffers.
    pub(crate) const ZERO: Self = Self(0);

    /// Snaps a device-space `f32` to the grid, the crate's one quantization
    /// point. Rejects non-finite inputs and coordinates outside
    /// [`COORD_LIMIT_PX`] with a typed error instead of wrapping.
    pub(crate) fn snap(v: f32) -> Result<Self, RasterError> {
        if !v.is_finite() {
            return Err(RasterError::NonFinite);
        }
        let raw = libm::roundf(v * ONE as f32);
        if !(-(COORD_LIMIT_RAW as f32)..=COORD_LIMIT_RAW as f32).contains(&raw) {
            return Err(RasterError::CoordinateOutOfRange {
                limit_px: COORD_LIMIT_PX,
            });
        }
        Ok(Self(raw as i32))
    }

    /// The pixel center of column/row `p`, exactly on the grid.
    pub(crate) const fn pixel_center(p: i32) -> Self {
        Self(p * ONE + HALF)
    }

    /// Raw fixed-point units (1/256 px), widened for cross products.
    pub(crate) const fn raw(self) -> i64 {
        self.0 as i64
    }

    /// The exact `f32` value; snapped grid values are representable without
    /// loss for the accepted coordinate range.
    pub(crate) fn to_f32(self) -> f32 {
        self.0 as f32 / ONE as f32
    }
}

#[cfg(test)]
mod tests;
