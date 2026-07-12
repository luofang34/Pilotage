//! The reference semantic outcome for one corpus case.
//!
//! Every field is derived from the reference stack: the layer gate
//! ([`validate_layers`]), the borrowing decoder ([`SceneCmds`]), and the
//! rasterizer ([`crate::render`]). `framing_valid` and `framing_boundaries`
//! mirror the browser's `validateSceneStructure` so the golden pins what that
//! weaker framing gate must return; the strong gate and render outcomes are
//! reference-only facts the browser cannot recompute on arbitrary bytes.

#![allow(clippy::expect_used, clippy::panic)]

use std::string::{String, ToString};
use std::vec::Vec;

use pilotage_instrument_scene::{
    Cmd, DecodeError, LayerError, PaintMode, PointsRef, SceneCmds, validate_layers,
};

use super::corpus::CorpusEntry;
use crate::{FrameId, FramebufferDims, RasterError, render};

pub(super) struct Outcome {
    pub(super) framing_valid: bool,
    pub(super) decode_ok: bool,
    pub(super) decode_error: Option<String>,
    pub(super) command_trace: Option<Vec<String>>,
    pub(super) gate_verdict: &'static str,
    pub(super) gate_error: Option<String>,
    pub(super) unknown: Option<u32>,
    pub(super) present: Option<u8>,
    pub(super) layer_commands: Option<[u16; 6]>,
    pub(super) render_ok: bool,
    pub(super) render_error: Option<String>,
    pub(super) canvas_methods: Option<Vec<String>>,
    pub(super) framing_boundaries: Option<Vec<usize>>,
}

/// The layer-gate verdict and its reported facts.
struct Gate {
    verdict: &'static str,
    error: Option<String>,
    unknown: Option<u32>,
    present: Option<u8>,
    commands: Option<[u16; 6]>,
}

pub(super) fn outcome_of(entry: &CorpusEntry) -> Outcome {
    let bytes = &entry.bytes;
    let (decode_ok, decode_error, full_trace) = decode(bytes);
    let g = gate(bytes);
    let (render_ok, render_error) = render_outcome(bytes);
    let canvas = entry.trace && g.verdict == "accept" && decode_ok && entry.category != "text";
    Outcome {
        framing_valid: framing_valid(bytes),
        decode_ok,
        decode_error,
        command_trace: if entry.trace { full_trace } else { None },
        gate_verdict: g.verdict,
        gate_error: g.error,
        unknown: g.unknown,
        present: g.present,
        layer_commands: g.commands,
        render_ok,
        render_error,
        canvas_methods: if canvas {
            Some(canvas_methods(bytes))
        } else {
            None
        },
        framing_boundaries: if entry.sweep {
            Some(boundaries(bytes))
        } else {
            None
        },
    }
}

/// Mirrors the browser's `validateSceneStructure`: version byte plus a walk by
/// declared command lengths that must land exactly at the end.
pub(super) fn framing_valid(bytes: &[u8]) -> bool {
    if bytes.first() != Some(&1) {
        return false;
    }
    let mut at = 1usize;
    while at + 3 <= bytes.len() {
        let plen = u16::from_le_bytes([bytes[at + 1], bytes[at + 2]]) as usize;
        at += 3 + plen;
    }
    at == bytes.len()
}

fn boundaries(bytes: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    if bytes.first() != Some(&1) {
        return out;
    }
    out.push(1);
    let mut at = 1usize;
    while at + 3 <= bytes.len() {
        let plen = u16::from_le_bytes([bytes[at + 1], bytes[at + 2]]) as usize;
        at += 3 + plen;
        if at <= bytes.len() {
            out.push(at);
        } else {
            break;
        }
    }
    out
}

fn decode(bytes: &[u8]) -> (bool, Option<String>, Option<Vec<String>>) {
    let cmds = match SceneCmds::new(bytes) {
        Ok(c) => c,
        Err(e) => return (false, Some(decode_class(&e)), None),
    };
    let mut trace = Vec::new();
    for item in cmds {
        match item {
            Ok(cmd) => trace.push(command_token(&cmd)),
            Err(e) => return (false, Some(decode_class(&e)), None),
        }
    }
    (true, None, Some(trace))
}

fn gate(bytes: &[u8]) -> Gate {
    match validate_layers(bytes) {
        Ok(r) => Gate {
            verdict: "accept",
            error: None,
            unknown: Some(r.unknown),
            present: Some(r.present),
            commands: Some(r.commands),
        },
        Err(e) => Gate {
            verdict: "reject",
            error: Some(layer_class(&e)),
            unknown: None,
            present: None,
            commands: None,
        },
    }
}

fn render_outcome(bytes: &[u8]) -> (bool, Option<String>) {
    let (w, h) = (64u32, 64u32);
    let mut fb = std::vec![0u8; (w * h * 4) as usize];
    match render(
        bytes,
        &mut fb,
        FramebufferDims::tight(w, h),
        FrameId::default(),
    ) {
        Ok(_) => (true, None),
        Err(e) => (false, Some(raster_class(&e))),
    }
}

fn q(v: f32) -> String {
    let d = v as f64;
    if d.is_nan() {
        "nan".to_string()
    } else if d.is_infinite() {
        if d > 0.0 {
            "inf".to_string()
        } else {
            "-inf".to_string()
        }
    } else {
        ((d * 256.0).floor() as i64).to_string()
    }
}

fn qf(v: f64) -> String {
    ((v * 256.0).floor() as i64).to_string()
}

fn mode_letters(mode: PaintMode) -> &'static str {
    match mode {
        PaintMode::Fill => "F",
        PaintMode::Stroke => "S",
        PaintMode::FillStroke => "FS",
    }
}

fn points_csv(points: &PointsRef<'_>) -> String {
    let mut parts = Vec::new();
    for [x, y] in points.iter() {
        parts.push(q(x));
        parts.push(q(y));
    }
    parts.join(",")
}

fn hex(text: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::new();
    for b in text.as_bytes() {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn command_token(cmd: &Cmd<'_>) -> String {
    match cmd {
        Cmd::Save => "01".to_string(),
        Cmd::Restore => "02".to_string(),
        Cmd::Translate { x, y } => std::format!("03:{},{}", q(*x), q(*y)),
        Cmd::Rotate { radians } => std::format!("04:{}", q(*radians)),
        Cmd::FillColor { color } => {
            std::format!("10:{},{},{},{}", color.r, color.g, color.b, color.a)
        }
        Cmd::Stroke { color, width } => {
            std::format!(
                "11:{},{},{},{},{}",
                color.r,
                color.g,
                color.b,
                color.a,
                q(*width)
            )
        }
        Cmd::Line { x1, y1, x2, y2 } => {
            std::format!("20:{},{},{},{}", q(*x1), q(*y1), q(*x2), q(*y2))
        }
        Cmd::Polyline { points } => std::format!("21:{}", points_csv(points)),
        Cmd::Polygon { mode, points } => {
            std::format!("22:{}:{}", mode_letters(*mode), points_csv(points))
        }
        Cmd::Rect { mode, x, y, w, h } => {
            std::format!(
                "23:{},{},{},{},{}",
                mode_letters(*mode),
                q(*x),
                q(*y),
                q(*w),
                q(*h)
            )
        }
        Cmd::Circle { mode, cx, cy, r } => {
            std::format!("24:{},{},{},{}", mode_letters(*mode), q(*cx), q(*cy), q(*r))
        }
        Cmd::Arc {
            cx,
            cy,
            r,
            start,
            sweep,
        } => {
            std::format!(
                "25:{},{},{},{},{}",
                q(*cx),
                q(*cy),
                q(*r),
                q(*start),
                q(*sweep)
            )
        }
        Cmd::Text {
            x,
            y,
            size,
            anchor,
            text,
        } => {
            std::format!(
                "30:{},{},{},{},{}",
                q(*size),
                anchor.to_u8(),
                q(*x),
                q(*y),
                hex(text)
            )
        }
        Cmd::ClipRect { x, y, w, h } => std::format!("40:{},{},{},{}", q(*x), q(*y), q(*w), q(*h)),
        Cmd::BeginLayer { layer } => std::format!("50:{}", layer.to_u8()),
        Cmd::EndLayer { layer } => std::format!("51:{}", layer.to_u8()),
        Cmd::Unknown { opcode } => std::format!("unknown:{}", opcode),
    }
}

fn canvas_methods(bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let cmds = SceneCmds::new(bytes).expect("decodable");
    for item in cmds {
        canvas_tokens(&item.expect("decodable"), &mut out);
    }
    out
}

fn fills(m: PaintMode) -> bool {
    matches!(m, PaintMode::Fill | PaintMode::FillStroke)
}

fn strokes(m: PaintMode) -> bool {
    matches!(m, PaintMode::Stroke | PaintMode::FillStroke)
}

fn paint_ops(mode: PaintMode, out: &mut Vec<String>) {
    if fills(mode) {
        out.push("fill".to_string());
    }
    if strokes(mode) {
        out.push("stroke".to_string());
    }
}

fn canvas_tokens(cmd: &Cmd<'_>, out: &mut Vec<String>) {
    match cmd {
        Cmd::Save => out.push("save".to_string()),
        Cmd::Restore => out.push("restore".to_string()),
        Cmd::Translate { x, y } => out.push(std::format!("translate:{},{}", q(*x), q(*y))),
        Cmd::Rotate { radians } => out.push(std::format!("rotate:{}", q(*radians))),
        Cmd::Line { x1, y1, x2, y2 } => {
            out.push("beginPath".to_string());
            out.push(std::format!("moveTo:{},{}", q(*x1), q(*y1)));
            out.push(std::format!("lineTo:{},{}", q(*x2), q(*y2)));
            out.push("stroke".to_string());
        }
        Cmd::Polyline { points } => {
            out.push("beginPath".to_string());
            path_points(points, out);
            out.push("stroke".to_string());
        }
        Cmd::Polygon { mode, points } => {
            out.push("beginPath".to_string());
            path_points(points, out);
            out.push("closePath".to_string());
            paint_ops(*mode, out);
        }
        Cmd::Rect { mode, x, y, w, h } => {
            let args = std::format!("{},{},{},{}", q(*x), q(*y), q(*w), q(*h));
            if fills(*mode) {
                out.push(std::format!("fillRect:{args}"));
            }
            if strokes(*mode) {
                out.push(std::format!("strokeRect:{args}"));
            }
        }
        Cmd::Circle { mode, cx, cy, r } => canvas_circle(*mode, *cx, *cy, *r, out),
        Cmd::Arc {
            cx,
            cy,
            r,
            start,
            sweep,
        } => canvas_arc(*cx, *cy, *r, *start, *sweep, out),
        Cmd::ClipRect { x, y, w, h } => {
            out.push("beginPath".to_string());
            out.push(std::format!("rect:{},{},{},{}", q(*x), q(*y), q(*w), q(*h)));
            out.push("clip".to_string());
        }
        Cmd::FillColor { .. }
        | Cmd::Stroke { .. }
        | Cmd::Text { .. }
        | Cmd::BeginLayer { .. }
        | Cmd::EndLayer { .. }
        | Cmd::Unknown { .. } => {}
    }
}

fn canvas_circle(mode: PaintMode, cx: f32, cy: f32, r: f32, out: &mut Vec<String>) {
    out.push("beginPath".to_string());
    out.push(std::format!(
        "arc:{},{},{},0,{}",
        q(cx),
        q(cy),
        q(r),
        qf(core::f64::consts::PI * 2.0)
    ));
    paint_ops(mode, out);
}

fn canvas_arc(cx: f32, cy: f32, r: f32, start: f32, sweep: f32, out: &mut Vec<String>) {
    out.push("beginPath".to_string());
    let end = start as f64 + sweep as f64;
    let anti = i32::from((sweep as f64) < 0.0);
    out.push(std::format!(
        "arc:{},{},{},{},{},{}",
        q(cx),
        q(cy),
        q(r),
        q(start),
        qf(end),
        anti
    ));
    out.push("stroke".to_string());
}

fn path_points(points: &PointsRef<'_>, out: &mut Vec<String>) {
    for (i, [x, y]) in points.iter().enumerate() {
        if i == 0 {
            out.push(std::format!("moveTo:{},{}", q(x), q(y)));
        } else {
            out.push(std::format!("lineTo:{},{}", q(x), q(y)));
        }
    }
}

fn decode_class(e: &DecodeError) -> String {
    match e {
        DecodeError::BadVersion { .. } => "BadVersion",
        DecodeError::Truncated => "Truncated",
        DecodeError::BadPayload { .. } => "BadPayload",
    }
    .to_string()
}

fn layer_class(e: &LayerError) -> String {
    match e {
        LayerError::Decode(d) => return std::format!("Decode:{}", decode_class(d)),
        LayerError::DuplicateLayer { .. } => "DuplicateLayer",
        LayerError::OutOfOrder { .. } => "OutOfOrder",
        LayerError::NestedLayer { .. } => "NestedLayer",
        LayerError::EndWithoutBegin { .. } => "EndWithoutBegin",
        LayerError::EndMismatch { .. } => "EndMismatch",
        LayerError::UnclosedLayer { .. } => "UnclosedLayer",
        LayerError::CommandOutsideLayer => "CommandOutsideLayer",
        LayerError::UnisolatedState { .. } => "UnisolatedState",
        LayerError::UnbalancedState { .. } => "UnbalancedState",
        LayerError::StackOverCapacity { .. } => "StackOverCapacity",
        LayerError::OverCapacity { .. } => "OverCapacity",
        LayerError::SceneTooLarge { .. } => "SceneTooLarge",
    }
    .to_string()
}

fn raster_class(e: &RasterError) -> String {
    match e {
        RasterError::Glyph(_) => "Glyph",
        RasterError::ZeroFramebuffer => "ZeroFramebuffer",
        RasterError::FramebufferTooLarge { .. } => "FramebufferTooLarge",
        RasterError::StrideTooSmall { .. } => "StrideTooSmall",
        RasterError::FramebufferTooSmall { .. } => "FramebufferTooSmall",
        RasterError::Layer(l) => return std::format!("Layer:{}", layer_class(l)),
        RasterError::Decode(d) => return std::format!("Decode:{}", decode_class(d)),
        RasterError::NonFinite => "NonFinite",
        RasterError::CoordinateOutOfRange { .. } => "CoordinateOutOfRange",
        RasterError::TooManyVertices { .. } => "TooManyVertices",
        RasterError::StackOverflow { .. } => "StackOverflow",
        RasterError::UnbalancedRestore => "UnbalancedRestore",
    }
    .to_string()
}
