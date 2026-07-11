//! Borrowing scene decoder for Rust backends and tests.

use crate::SCENE_FORMAT_VERSION;
use crate::cmd::{Anchor, Cmd, PaintMode, PointsRef};
use crate::color::Rgba8;
use crate::layer::LayerId;
use crate::opcode;

/// Why decoding stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// The scene does not start with a version this decoder reads.
    BadVersion {
        /// The version byte found.
        found: u8,
    },
    /// The bytes end mid-command.
    Truncated,
    /// A command payload is malformed for its opcode.
    BadPayload {
        /// The opcode whose payload was malformed.
        opcode: u8,
    },
}

/// Iterator over the commands of an encoded scene.
///
/// Unknown opcodes decode as [`Cmd::Unknown`] (their payload is skipped
/// using the encoded length) so newer scenes degrade gracefully on older
/// backends; malformed payloads for *known* opcodes are hard errors.
#[derive(Debug, Clone)]
pub struct SceneCmds<'a> {
    rest: &'a [u8],
    failed: bool,
}

impl<'a> SceneCmds<'a> {
    /// Starts decoding; checks the version byte.
    pub fn new(scene: &'a [u8]) -> Result<Self, DecodeError> {
        match scene.split_first() {
            Some((&v, rest)) if v == SCENE_FORMAT_VERSION => Ok(Self {
                rest,
                failed: false,
            }),
            Some((&v, _)) => Err(DecodeError::BadVersion { found: v }),
            None => Err(DecodeError::Truncated),
        }
    }

    /// Bytes not yet consumed. `scene.len() - remaining()` is the offset
    /// of the next command, which is how [`crate::validate_layers`]
    /// reports per-layer byte ranges without allocating.
    pub fn remaining(&self) -> usize {
        self.rest.len()
    }

    fn take_cmd(&mut self) -> Result<Cmd<'a>, DecodeError> {
        let (&op, after_op) = self.rest.split_first().ok_or(DecodeError::Truncated)?;
        let (len_bytes, after_len) = split_n(after_op, 2).ok_or(DecodeError::Truncated)?;
        let plen = u16::from_le_bytes([len_bytes[0], len_bytes[1]]) as usize;
        let (payload, rest) = split_n(after_len, plen).ok_or(DecodeError::Truncated)?;
        self.rest = rest;
        decode_payload(op, payload)
    }
}

impl<'a> Iterator for SceneCmds<'a> {
    type Item = Result<Cmd<'a>, DecodeError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.rest.is_empty() {
            return None;
        }
        let item = self.take_cmd();
        if item.is_err() {
            self.failed = true;
        }
        Some(item)
    }
}

fn split_n(bytes: &[u8], n: usize) -> Option<(&[u8], &[u8])> {
    if bytes.len() < n {
        None
    } else {
        Some(bytes.split_at(n))
    }
}

fn f32_at(payload: &[u8], i: usize) -> Option<f32> {
    let at = i.checked_mul(4)?;
    let b = payload.get(at..at.checked_add(4)?)?;
    Some(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn u32_at(payload: &[u8], byte_at: usize) -> Option<u32> {
    let b = payload.get(byte_at..byte_at.checked_add(4)?)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn decode_payload(op: u8, payload: &[u8]) -> Result<Cmd<'_>, DecodeError> {
    let bad = DecodeError::BadPayload { opcode: op };
    match op {
        opcode::SAVE => Ok(Cmd::Save),
        opcode::RESTORE => Ok(Cmd::Restore),
        opcode::TRANSLATE => Ok(Cmd::Translate {
            x: f32_at(payload, 0).ok_or(bad)?,
            y: f32_at(payload, 1).ok_or(bad)?,
        }),
        opcode::ROTATE => Ok(Cmd::Rotate {
            radians: f32_at(payload, 0).ok_or(bad)?,
        }),
        opcode::FILL_COLOR => Ok(Cmd::FillColor {
            color: Rgba8::from_u32(u32_at(payload, 0).ok_or(bad)?),
        }),
        opcode::STROKE => Ok(Cmd::Stroke {
            color: Rgba8::from_u32(u32_at(payload, 0).ok_or(bad)?),
            width: f32_at(payload, 1).ok_or(bad)?,
        }),
        opcode::LINE => Ok(Cmd::Line {
            x1: f32_at(payload, 0).ok_or(bad)?,
            y1: f32_at(payload, 1).ok_or(bad)?,
            x2: f32_at(payload, 2).ok_or(bad)?,
            y2: f32_at(payload, 3).ok_or(bad)?,
        }),
        opcode::POLYLINE => Ok(Cmd::Polyline {
            points: PointsRef::from_bytes(payload).ok_or(bad)?,
        }),
        opcode::POLYGON => {
            let (&mode, pts) = payload.split_first().ok_or(bad)?;
            Ok(Cmd::Polygon {
                mode: PaintMode::from_u8(mode).ok_or(bad)?,
                points: PointsRef::from_bytes(pts).ok_or(bad)?,
            })
        }
        opcode::RECT => {
            let (&mode, xs) = payload.split_first().ok_or(bad)?;
            Ok(Cmd::Rect {
                mode: PaintMode::from_u8(mode).ok_or(bad)?,
                x: f32_at(xs, 0).ok_or(bad)?,
                y: f32_at(xs, 1).ok_or(bad)?,
                w: f32_at(xs, 2).ok_or(bad)?,
                h: f32_at(xs, 3).ok_or(bad)?,
            })
        }
        opcode::CIRCLE => {
            let (&mode, xs) = payload.split_first().ok_or(bad)?;
            Ok(Cmd::Circle {
                mode: PaintMode::from_u8(mode).ok_or(bad)?,
                cx: f32_at(xs, 0).ok_or(bad)?,
                cy: f32_at(xs, 1).ok_or(bad)?,
                r: f32_at(xs, 2).ok_or(bad)?,
            })
        }
        opcode::ARC => Ok(Cmd::Arc {
            cx: f32_at(payload, 0).ok_or(bad)?,
            cy: f32_at(payload, 1).ok_or(bad)?,
            r: f32_at(payload, 2).ok_or(bad)?,
            start: f32_at(payload, 3).ok_or(bad)?,
            sweep: f32_at(payload, 4).ok_or(bad)?,
        }),
        opcode::TEXT => {
            let size = f32_at(payload, 0).ok_or(bad)?;
            let anchor = Anchor::from_u8(*payload.get(4).ok_or(bad)?).ok_or(bad)?;
            let x = u32_at(payload, 5).map(f32::from_bits).ok_or(bad)?;
            let y = u32_at(payload, 9).map(f32::from_bits).ok_or(bad)?;
            let text_bytes = payload.get(13..).ok_or(bad)?;
            let text = core::str::from_utf8(text_bytes).map_err(|_| bad)?;
            Ok(Cmd::Text {
                x,
                y,
                size,
                anchor,
                text,
            })
        }
        opcode::CLIP_RECT => Ok(Cmd::ClipRect {
            x: f32_at(payload, 0).ok_or(bad)?,
            y: f32_at(payload, 1).ok_or(bad)?,
            w: f32_at(payload, 2).ok_or(bad)?,
            h: f32_at(payload, 3).ok_or(bad)?,
        }),
        // An unknown layer *id* is a hard error, unlike an unknown
        // opcode: content whose criticality cannot be placed must not
        // be painted (REN-01). New layer ids require a version bump.
        opcode::BEGIN_LAYER => Ok(Cmd::BeginLayer {
            layer: LayerId::from_u8(*payload.first().ok_or(bad)?).ok_or(bad)?,
        }),
        opcode::END_LAYER => Ok(Cmd::EndLayer {
            layer: LayerId::from_u8(*payload.first().ok_or(bad)?).ok_or(bad)?,
        }),
        other => Ok(Cmd::Unknown { opcode: other }),
    }
}

#[cfg(test)]
mod tests;
