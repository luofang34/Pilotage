//! The render entry point: validate, clear, paint, and fail visibly.
//!
//! [`render`] gates the frame on
//! [`validate_layers`](pilotage_instrument_scene::validate_layers) exactly as
//! the WASM backend does, clears to the defined initial state, then walks the
//! command stream. Layer markers are no-ops for painting; the save/restore
//! they wrap are honored as ordinary commands. Any failure once the
//! framebuffer geometry is accepted spoils the whole frame before returning,
//! so a caller can never mistake stale contents for a fresh render.

use pilotage_instrument_scene::{Anchor, Cmd, PaintMode, PointsRef, SceneCmds, validate_layers};

use crate::curve::{self, Disc};
use crate::error::RasterError;
use crate::fixed::Fx;
use crate::paint::fill_polygon;
use crate::report::{FrameId, FramebufferDims, RenderReport, RenderStatus};
use crate::state::RenderState;
use crate::stroke::stroke_path;
use crate::surface::Surface;
use crate::text::{self, Run};
use crate::transform::Affine;

/// Renders an encoded scene into a caller-provided RGBA8 framebuffer.
///
/// `frame` is echoed into the report unchanged (the rasterizer is stateless).
/// On success the framebuffer holds the painted frame; on any error after the
/// framebuffer geometry is accepted, it holds the spoil pattern and the error
/// is returned. Geometry errors leave the buffer untouched. See the crate
/// docs for the full pixel-format, compositing, and coverage contract.
pub fn render(
    scene: &[u8],
    pixels: &mut [u8],
    dims: FramebufferDims,
    frame: FrameId,
) -> Result<RenderReport, RasterError> {
    let mut surface = Surface::new(pixels, dims)?;
    match paint(scene, &mut surface) {
        Ok((unknown_opcodes, layers_present)) => Ok(RenderReport {
            scene_version: scene.first().copied().unwrap_or(0),
            status: RenderStatus::Painted,
            frame,
            unknown_opcodes,
            layers_present,
            work: surface.work(),
        }),
        Err(error) => {
            surface.spoil();
            Err(error)
        }
    }
}

/// Validates, clears, and paints; returns the unknown-opcode count and the
/// present-layer bitset for the report.
fn paint(scene: &[u8], surface: &mut Surface<'_>) -> Result<(u32, u8), RasterError> {
    let report = validate_layers(scene)?;
    // The controlled glyph pack is part of the display's integrity
    // envelope: a corrupt pack fails the frame before any text could
    // paint from it (REN-02's no-fallback rule).
    pilotage_instrument_glyphs::PANEL_GLYPHS.verify()?;
    surface.clear();
    let mut state = RenderState::new(surface.bounds());
    let mut unknown: u32 = 0;
    for item in SceneCmds::new(scene)? {
        run(&item?, &mut state, surface, &mut unknown)?;
    }
    Ok((unknown, report.present))
}

fn run(
    cmd: &Cmd<'_>,
    state: &mut RenderState,
    surface: &mut Surface<'_>,
    unknown: &mut u32,
) -> Result<(), RasterError> {
    match cmd {
        Cmd::Save => state.save(),
        Cmd::Restore => state.restore(),
        Cmd::Translate { x, y } => translate(state, *x, *y),
        Cmd::Rotate { radians } => rotate(state, *radians),
        Cmd::FillColor { color } => {
            state.set_fill(*color);
            Ok(())
        }
        Cmd::Stroke { color, width } => state.set_stroke(*color, *width),
        Cmd::ClipRect { x, y, w, h } => clip(state, *x, *y, *w, *h),
        Cmd::Line { x1, y1, x2, y2 } => line(state, surface, [*x1, *y1, *x2, *y2]),
        Cmd::Polyline { points } => polyline(state, surface, points),
        Cmd::Polygon { mode, points } => polygon(state, surface, *mode, points),
        Cmd::Rect { mode, x, y, w, h } => rect(state, surface, *mode, [*x, *y, *w, *h]),
        Cmd::Circle { mode, cx, cy, r } => circle(state, surface, *mode, [*cx, *cy, *r]),
        Cmd::Arc {
            cx,
            cy,
            r,
            start,
            sweep,
        } => arc(state, surface, [*cx, *cy, *r], *start, *sweep),
        Cmd::Text {
            x,
            y,
            size,
            anchor,
            text: run_text,
        } => draw_text(state, surface, [*x, *y, *size], *anchor, run_text),
        Cmd::BeginLayer { .. } | Cmd::EndLayer { .. } => Ok(()),
        Cmd::Unknown { .. } => {
            *unknown = unknown.wrapping_add(1);
            Ok(())
        }
    }
}

fn finite(v: f32) -> Result<f32, RasterError> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(RasterError::NonFinite)
    }
}

fn fills(mode: PaintMode) -> bool {
    matches!(mode, PaintMode::Fill | PaintMode::FillStroke)
}

fn strokes(mode: PaintMode) -> bool {
    matches!(mode, PaintMode::Stroke | PaintMode::FillStroke)
}

fn translate(state: &mut RenderState, x: f32, y: f32) -> Result<(), RasterError> {
    state.ctm_mut().translate(finite(x)?, finite(y)?);
    Ok(())
}

fn rotate(state: &mut RenderState, radians: f32) -> Result<(), RasterError> {
    state.ctm_mut().rotate(finite(radians)?);
    Ok(())
}

fn clip(state: &mut RenderState, x: f32, y: f32, w: f32, h: f32) -> Result<(), RasterError> {
    let ctm = state.current().ctm;
    let corners = rect_corners(&ctm, x, y, w, h)?;
    state.clip_rect(&corners);
    Ok(())
}

fn line(state: &RenderState, surface: &mut Surface<'_>, v: [f32; 4]) -> Result<(), RasterError> {
    let gs = state.current();
    let verts = [gs.ctm.map(v[0], v[1])?, gs.ctm.map(v[2], v[3])?];
    stroke_path(
        surface,
        gs.clip,
        &verts,
        false,
        gs.stroke_width,
        gs.stroke_color,
    );
    Ok(())
}

fn polyline(
    state: &RenderState,
    surface: &mut Surface<'_>,
    points: &PointsRef<'_>,
) -> Result<(), RasterError> {
    let gs = state.current();
    let mut buf = [[Fx::ZERO; 2]; crate::MAX_POLYGON_VERTICES];
    let n = map_points(&gs.ctm, points, &mut buf)?;
    stroke_path(
        surface,
        gs.clip,
        &buf[..n],
        false,
        gs.stroke_width,
        gs.stroke_color,
    );
    Ok(())
}

fn polygon(
    state: &RenderState,
    surface: &mut Surface<'_>,
    mode: PaintMode,
    points: &PointsRef<'_>,
) -> Result<(), RasterError> {
    let gs = state.current();
    let mut buf = [[Fx::ZERO; 2]; crate::MAX_POLYGON_VERTICES];
    let n = map_points(&gs.ctm, points, &mut buf)?;
    if fills(mode) {
        fill_polygon(surface, gs.clip, &buf[..n], gs.fill);
    }
    if strokes(mode) {
        stroke_path(
            surface,
            gs.clip,
            &buf[..n],
            true,
            gs.stroke_width,
            gs.stroke_color,
        );
    }
    Ok(())
}

fn rect(
    state: &RenderState,
    surface: &mut Surface<'_>,
    mode: PaintMode,
    v: [f32; 4],
) -> Result<(), RasterError> {
    let gs = state.current();
    let corners = rect_corners(&gs.ctm, v[0], v[1], v[2], v[3])?;
    if fills(mode) {
        fill_polygon(surface, gs.clip, &corners, gs.fill);
    }
    if strokes(mode) {
        stroke_path(
            surface,
            gs.clip,
            &corners,
            true,
            gs.stroke_width,
            gs.stroke_color,
        );
    }
    Ok(())
}

fn circle(
    state: &RenderState,
    surface: &mut Surface<'_>,
    mode: PaintMode,
    v: [f32; 3],
) -> Result<(), RasterError> {
    let gs = state.current();
    let disc = disc(&gs.ctm, v[0], v[1], v[2])?;
    if fills(mode) {
        curve::fill_circle(surface, gs.clip, disc, gs.fill);
    }
    if strokes(mode) {
        curve::stroke_circle(
            surface,
            gs.clip,
            disc,
            gs.stroke_width / 2.0,
            gs.stroke_color,
        );
    }
    Ok(())
}

fn arc(
    state: &RenderState,
    surface: &mut Surface<'_>,
    v: [f32; 3],
    start: f32,
    sweep: f32,
) -> Result<(), RasterError> {
    let gs = state.current();
    let disc = disc(&gs.ctm, v[0], v[1], v[2])?;
    let device_start = finite(start)? + gs.ctm.rotation();
    curve::stroke_arc(
        surface,
        gs.clip,
        disc,
        device_start,
        finite(sweep)?,
        gs.stroke_width / 2.0,
        gs.stroke_color,
    );
    Ok(())
}

fn draw_text(
    state: &RenderState,
    surface: &mut Surface<'_>,
    v: [f32; 3],
    anchor: Anchor,
    run_text: &str,
) -> Result<(), RasterError> {
    let gs = state.current();
    let run = Run {
        x: v[0],
        y: v[1],
        size: finite(v[2])?,
        anchor,
        text: run_text,
    };
    text::draw_run(surface, gs.clip, &gs.ctm, run, gs.fill)
}

/// Transforms the four corners of a logical rectangle to snapped device
/// coordinates in clockwise order.
fn rect_corners(ctm: &Affine, x: f32, y: f32, w: f32, h: f32) -> Result<[[Fx; 2]; 4], RasterError> {
    Ok([
        ctm.map(x, y)?,
        ctm.map(x + w, y)?,
        ctm.map(x + w, y + h)?,
        ctm.map(x, y + h)?,
    ])
}

/// Maps a circle/arc center and radius into device space; a non-positive or
/// non-finite radius paints nothing (radius zero).
fn disc(ctm: &Affine, cx: f32, cy: f32, r: f32) -> Result<Disc, RasterError> {
    let center = ctm.map(cx, cy)?;
    let radius = Fx::snap(finite(r)?.max(0.0))?;
    Ok(Disc {
        cx: center[0].to_f32(),
        cy: center[1].to_f32(),
        r: radius.to_f32(),
    })
}

fn map_points(
    ctm: &Affine,
    points: &PointsRef<'_>,
    buf: &mut [[Fx; 2]; crate::MAX_POLYGON_VERTICES],
) -> Result<usize, RasterError> {
    let mut n = 0;
    for [x, y] in points.iter() {
        let slot = buf.get_mut(n).ok_or(RasterError::TooManyVertices {
            limit: crate::MAX_POLYGON_VERTICES,
        })?;
        *slot = ctm.map(x, y)?;
        n = n.wrapping_add(1);
    }
    Ok(n)
}

#[cfg(test)]
mod tests;
