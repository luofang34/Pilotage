//! Prediction of the browser interpreter's raw-argument guards.
//!
//! `instruments.js` refuses to hand Canvas2D non-finite or out-of-range
//! geometry: `interpretScene` guards every float that would become Canvas
//! geometry and every path against the shared vertex budget, throwing before
//! the drawing call. This module mirrors that rule command for command so the
//! golden pins exactly where the browser must throw (`interpreterRejects`),
//! and the browser test fails with `GuardMissing` if a predicted rejection
//! paints instead.

#![allow(clippy::expect_used, clippy::panic)]

use std::string::{String, ToString};

use pilotage_instrument_scene::{Cmd, SceneCmds};

/// Mirrors the browser interpreter's raw-argument guards: every float that
/// would reach Canvas geometry must be finite, coordinates and sizes must
/// satisfy |v| <= COORD_LIMIT_PX, arc angles need only be finite, and a path
/// may carry at most the shared vertex budget. Evaluation order matches
/// `interpretScene` so the first rejection reason is the one the browser
/// reports. This is deliberately a RAW-argument rule: the reference
/// rasterizer range-checks in device space after the transform, which the
/// interpreter cannot reproduce without becoming a rasterizer itself.
pub(super) fn interpreter_rejects(bytes: &[u8]) -> Option<String> {
    let cmds = SceneCmds::new(bytes).ok()?;
    for item in cmds {
        let reason = command_rejection(&item.ok()?);
        if reason.is_some() {
            return reason;
        }
    }
    None
}

fn coord_ok(v: f32) -> bool {
    let d = v as f64;
    d.is_finite() && d.abs() <= crate::fixed::COORD_LIMIT_PX as f64
}

fn coords_ok(vs: &[f32]) -> bool {
    vs.iter().all(|&v| coord_ok(v))
}

fn command_rejection(cmd: &Cmd<'_>) -> Option<String> {
    let coordinate = || Some("coordinate".to_string());
    match cmd {
        Cmd::Translate { x, y } if !coords_ok(&[*x, *y]) => coordinate(),
        Cmd::Rotate { radians } if !(*radians as f64).is_finite() => Some("angle".to_string()),
        Cmd::Stroke { width, .. } if !coord_ok(*width) => coordinate(),
        Cmd::Line { x1, y1, x2, y2 } if !coords_ok(&[*x1, *y1, *x2, *y2]) => coordinate(),
        Cmd::Polyline { points } | Cmd::Polygon { points, .. } => {
            if points.iter().count() > crate::MAX_POLYGON_VERTICES {
                return Some("vertex-count".to_string());
            }
            for [x, y] in points.iter() {
                if !coords_ok(&[x, y]) {
                    return coordinate();
                }
            }
            None
        }
        Cmd::Rect { x, y, w, h, .. } | Cmd::ClipRect { x, y, w, h }
            if !coords_ok(&[*x, *y, *w, *h]) =>
        {
            coordinate()
        }
        Cmd::Circle { cx, cy, r, .. } if !coords_ok(&[*cx, *cy, *r]) => coordinate(),
        Cmd::Arc {
            cx,
            cy,
            r,
            start,
            sweep,
        } => {
            if !coords_ok(&[*cx, *cy, *r]) {
                return coordinate();
            }
            let (s, e) = (*start as f64, *start as f64 + *sweep as f64);
            if !s.is_finite() || !e.is_finite() {
                return Some("angle".to_string());
            }
            None
        }
        Cmd::Text { x, y, size, .. } if !coords_ok(&[*x, *y, *size]) => coordinate(),
        _ => None,
    }
}
