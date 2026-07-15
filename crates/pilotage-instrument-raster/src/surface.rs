//! The RGBA8 device surface: geometry, clipping, compositing, and spoiling.
//!
//! Pixels are straight-alpha sRGB (R, G, B, A), row-major, top-left origin.
//! Compositing is source-over performed directly on the sRGB-encoded 8-bit
//! channels — a defined, colorimetrically simplified rule chosen so a frame
//! is bit-reproducible on every target, with all division rounded to nearest
//! by integer arithmetic. Coverage is sampled once per pixel per primitive,
//! so a translucent shape never composites a pixel against itself.

use crate::error::RasterError;
use crate::report::FramebufferDims;

/// A half-open pixel rectangle used for clip regions and shape bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PixelRect {
    /// Inclusive left column.
    pub(crate) left: i32,
    /// Inclusive top row.
    pub(crate) top: i32,
    /// Exclusive right column.
    pub(crate) right: i32,
    /// Exclusive bottom row.
    pub(crate) bottom: i32,
}

impl PixelRect {
    /// The intersection of two rectangles, possibly empty.
    pub(crate) fn intersect(self, other: Self) -> Self {
        Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        }
    }

    /// Whether the rectangle covers no pixels.
    pub(crate) fn is_empty(self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }
}

/// An sRGB straight-alpha RGBA8 render target borrowed from the caller.
pub(crate) struct Surface<'a> {
    pixels: &'a mut [u8],
    width: i32,
    height: i32,
    stride: usize,
    /// Coverage evaluations performed (one per pixel-center test in a
    /// primitive's bounded region loop). A pure function of scene and
    /// dimensions, so it doubles as the target-independent work metric.
    coverage_samples: u64,
    /// Integer winding edge tests inside polygon coverage samples.
    polygon_edge_tests: u64,
    /// f32 segment-distance tests inside stroke samples (worst case).
    stroke_segment_tests: u64,
    /// Center-distance tests inside circle/arc samples.
    disc_tests: u64,
    /// Arc angular-membership extras beyond the disc test.
    arc_tests: u64,
    /// Source-over composites actually applied.
    composites: u64,
}

impl<'a> Surface<'a> {
    /// Validates framebuffer geometry and borrows the pixel slice. All
    /// failures here are raised before any pixel is touched.
    pub(crate) fn new(pixels: &'a mut [u8], dims: FramebufferDims) -> Result<Self, RasterError> {
        if dims.width == 0 || dims.height == 0 {
            return Err(RasterError::ZeroFramebuffer);
        }
        if dims.width > crate::MAX_DIMENSION || dims.height > crate::MAX_DIMENSION {
            return Err(RasterError::FramebufferTooLarge {
                width: dims.width,
                height: dims.height,
                limit: crate::MAX_DIMENSION,
            });
        }
        let min_row = dims.width as usize * 4;
        let stride = dims.stride_bytes as usize;
        if stride < min_row {
            return Err(RasterError::StrideTooSmall {
                stride_bytes: stride,
                min_bytes: min_row,
            });
        }
        let need = (dims.height as usize - 1) * stride + min_row;
        if pixels.len() < need {
            return Err(RasterError::FramebufferTooSmall {
                need,
                have: pixels.len(),
            });
        }
        Ok(Self {
            pixels,
            width: dims.width as i32,
            height: dims.height as i32,
            stride,
            coverage_samples: 0,
            polygon_edge_tests: 0,
            stroke_segment_tests: 0,
            disc_tests: 0,
            arc_tests: 0,
            composites: 0,
        })
    }

    /// The full-surface clip rectangle.
    pub(crate) fn bounds(&self) -> PixelRect {
        PixelRect {
            left: 0,
            top: 0,
            right: self.width,
            bottom: self.height,
        }
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return None;
        }
        Some(y as usize * self.stride + x as usize * 4)
    }

    /// Clears the frame region to transparent black, the defined initial
    /// state before any command paints.
    pub(crate) fn clear(&mut self) {
        let row_bytes = self.width as usize * 4;
        for y in 0..self.height {
            let start = y as usize * self.stride;
            if let Some(row) = self.pixels.get_mut(start..start + row_bytes) {
                row.fill(0);
            }
        }
    }

    /// Counts one pixel-center coverage evaluation.
    pub(crate) fn count_sample(&mut self) {
        self.coverage_samples = self.coverage_samples.wrapping_add(1);
    }

    /// Counts the integer winding edge tests of one polygon coverage sample
    /// (every edge — the count is the worst case the timing model prices).
    pub(crate) fn count_polygon_edge_tests(&mut self, n: u64) {
        self.polygon_edge_tests = self.polygon_edge_tests.wrapping_add(n);
    }

    /// Counts the f32 segment-distance tests of one stroke coverage sample
    /// (every segment, never an early-exit path).
    pub(crate) fn count_stroke_segment_tests(&mut self, n: u64) {
        self.stroke_segment_tests = self.stroke_segment_tests.wrapping_add(n);
    }

    /// Counts one circle/arc center-distance test.
    pub(crate) fn count_disc_tests(&mut self, n: u64) {
        self.disc_tests = self.disc_tests.wrapping_add(n);
    }

    /// Counts one arc angular-membership evaluation (cap distances, `atan2f`,
    /// `fmodf`) beyond the sample's disc test.
    pub(crate) fn count_arc_tests(&mut self, n: u64) {
        self.arc_tests = self.arc_tests.wrapping_add(n);
    }

    /// The work performed so far, by cost class.
    pub(crate) fn work(&self) -> crate::report::RenderWork {
        crate::report::RenderWork {
            coverage_samples: self.coverage_samples,
            polygon_edge_tests: self.polygon_edge_tests,
            stroke_segment_tests: self.stroke_segment_tests,
            disc_tests: self.disc_tests,
            arc_tests: self.arc_tests,
            composites: self.composites,
        }
    }

    /// Composites one straight-alpha sRGB pixel with source-over.
    pub(crate) fn composite(&mut self, x: i32, y: i32, color: [u8; 4]) {
        self.composites = self.composites.wrapping_add(1);
        let sa = color[3] as u32;
        if sa == 0 {
            return;
        }
        let Some(idx) = self.index(x, y) else {
            return;
        };
        let Some(px) = self.pixels.get_mut(idx..idx + 4) else {
            return;
        };
        if sa == 255 {
            px.copy_from_slice(&color);
            return;
        }
        let inv = 255 - sa;
        let da = px[3] as u32;
        let out_a = sa + div255(da * inv);
        let denom = out_a * 255;
        for c in 0..3 {
            let num = color[c] as u32 * sa * 255 + px[c] as u32 * da * inv;
            px[c] = ((num + denom / 2) / denom) as u8;
        }
        px[3] = out_a as u8;
    }

    /// Overwrites the entire frame with the unmistakable failure pattern:
    /// opaque black plus a red diagonal cross, by direct writes that cannot
    /// fail, so no plausible frame survives an error.
    pub(crate) fn spoil(&mut self) {
        for y in 0..self.height {
            for x in 0..self.width {
                self.put(x, y, [0, 0, 0, 255]);
            }
        }
        let thickness = 1 + self.width.min(self.height) / 128;
        for x in 0..self.width {
            let y = (x as i64 * (self.height - 1) as i64 / (self.width.max(2) - 1) as i64) as i32;
            for t in -thickness..=thickness {
                self.put(x, y + t, [255, 0, 0, 255]);
                self.put(x, self.height - 1 - y + t, [255, 0, 0, 255]);
            }
        }
    }

    fn put(&mut self, x: i32, y: i32, color: [u8; 4]) {
        if let Some(idx) = self.index(x, y)
            && let Some(px) = self.pixels.get_mut(idx..idx + 4)
        {
            px.copy_from_slice(&color);
        }
    }
}

/// Round-to-nearest division by 255 for a channel product.
fn div255(v: u32) -> u32 {
    ((v + 128) * 257) >> 16
}

#[cfg(test)]
mod tests;
