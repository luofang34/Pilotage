//! Deterministic software reference rasterizer for the instrument scene IR
//! (ADR-0017, REN-03).
//!
//! [`render`] paints an encoded [`pilotage_instrument_scene`] scene into a
//! caller-provided RGBA8 framebuffer with no GPU, OS, clock, random source,
//! allocation, or platform font dependency, producing bit-identical frames
//! across targets. The crate is `no_std` and allocation-free; every `f32`
//! operation goes through `libm` so results do not vary with the platform
//! math library, and IEEE-754 `f32` arithmetic makes them identical across
//! CI architectures.
//!
//! # Framebuffer contract
//! - Pixels are RGBA8 — four bytes per pixel in R, G, B, A order, straight
//!   (non-premultiplied) alpha, sRGB, row-major, top-left origin, with
//!   `stride_bytes` between row starts (see [`FramebufferDims`]).
//! - The frame is cleared to transparent black before the first command; a
//!   scene's own background layer paints any opaque backdrop.
//! - Compositing is source-over evaluated directly on the sRGB-encoded 8-bit
//!   channels (a defined, colorimetrically simplified rule for exact
//!   reproducibility), with all division rounded to nearest by integer math.
//!   Each pixel is covered at most once per primitive, so a translucent shape
//!   never composites against itself.
//!
//! # Rasterization contract
//! - Coverage is sampled at pixel centers `(x + 0.5, y + 0.5)` with no
//!   anti-aliasing: a pixel is wholly inside or wholly outside a shape.
//! - Every device coordinate is snapped once to Q8.8 fixed point (1/256 px)
//!   as it leaves the affine transform; that is the single quantization
//!   boundary, so a frame does not depend on transform rounding beyond it.
//! - Transforms translate/rotate post-multiply in command order (Canvas2D
//!   semantics) from an identity, 1:1 initial transform (one logical unit is
//!   one device pixel). The IR has no scale op, so this stays a rigid motion:
//!   stroke widths and radii are unscaled and a circle stays a circle.
//! - Clipping is rectangle intersection only; each clip is the device-space
//!   axis-aligned bound of the transformed rectangle (conservative under
//!   rotation).
//! - Filled polygons use the non-zero winding rule, computed in integer
//!   arithmetic. Strokes are centered with round joins and caps, evaluated as
//!   capsule distance to the path; circles and arcs use exact center
//!   distance. Text renders as a deterministic placeholder box derived only
//!   from the run's size, anchor, and byte length until REN-02's glyph pack
//!   replaces it.
//!
//! # Fail-safe behavior and bounds
//! - Non-finite coordinates or transforms, out-of-range coordinates,
//!   over-budget shapes, and stack overflow are typed [`RasterError`]s; a
//!   NaN or infinity is never painted.
//! - Any error raised after the framebuffer geometry is accepted spoils the
//!   whole frame — opaque black plus a red diagonal cross drawn by direct
//!   writes that cannot fail — so no plausible old frame survives a failure.
//!   Framebuffer-geometry errors leave the buffer untouched.
//! - Resource bounds are explicit: [`MAX_DIMENSION`] per axis,
//!   [`MAX_POLYGON_VERTICES`] per shape, and the scene crate's stack, command,
//!   and byte budgets. The worst-case frame size is [`WORST_CASE_FRAME_BYTES`].
//!
//! # Execution-time measurement
//! The renderer is straight-line over the scene and framebuffer with no I/O
//! and only documented-bounded loops, so a target-independent worst-case
//! execution time is a sum of bounded step counts — command dispatches (at
//! most [`pilotage_instrument_scene::MAX_LAYER_COMMANDS`] times the layer
//! count) and per-pixel coverage tests (at most framebuffer pixels times a
//! shape's edges) — multiplied by the selected target's per-step cycle bound.
//! A step-counting harness can wrap the coverage predicate and command
//! dispatch without changing output; measured cycle costs and the final WCET
//! wait for hardware selection and are out of scope here.

#![no_std]

#[cfg(test)]
extern crate std;

mod curve;
mod error;
mod fixed;
mod paint;
mod raster;
mod report;
mod state;
mod stroke;
mod surface;
mod text;
mod transform;

pub use error::RasterError;
pub use raster::render;
pub use report::{FrameId, FramebufferDims, RenderReport, RenderStatus, RenderWork};

/// Largest framebuffer dimension accepted per axis, in pixels.
pub const MAX_DIMENSION: u32 = 4096;

/// Largest vertex count a single polyline or polygon command may paint.
pub const MAX_POLYGON_VERTICES: usize = 512;

/// Worst-case frame size in bytes: a [`MAX_DIMENSION`]-square RGBA8 frame.
pub const WORST_CASE_FRAME_BYTES: usize = MAX_DIMENSION as usize * MAX_DIMENSION as usize * 4;
