//! Allocation-free scene encoder.

use crate::SCENE_FORMAT_VERSION;
use crate::cmd::{Anchor, MAX_TEXT_BYTES, PaintMode};
use crate::color::Rgba8;
use crate::opcode;

/// Why encoding stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneError {
    /// The output buffer has no room for the next command.
    BufferFull,
    /// A polyline/polygon has more vertices than one command can carry.
    TooManyPoints,
    /// A text run exceeds [`MAX_TEXT_BYTES`].
    TextTooLong,
}

/// Encodes drawing commands into a caller-provided byte buffer.
///
/// Wire layout: one version byte, then per command
/// `[opcode u8][payload_len u16 LE][payload]`. The explicit payload length
/// is what lets decoders skip opcodes they do not recognize. A command
/// that does not fit is rolled back whole: the buffer never ends with a
/// truncated command.
#[derive(Debug)]
pub struct SceneWriter<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl<'a> SceneWriter<'a> {
    /// Starts a scene in `buf`, writing the format version byte.
    pub fn new(buf: &'a mut [u8]) -> Result<Self, SceneError> {
        let mut w = Self { buf, len: 0 };
        w.put(&[SCENE_FORMAT_VERSION])?;
        Ok(w)
    }

    /// Bytes encoded so far.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether nothing beyond the version byte has been encoded.
    pub fn is_empty(&self) -> bool {
        self.len <= 1
    }

    /// Finishes the scene and returns the number of encoded bytes.
    pub fn finish(self) -> usize {
        self.len
    }

    fn put(&mut self, bytes: &[u8]) -> Result<(), SceneError> {
        let end = self
            .len
            .checked_add(bytes.len())
            .ok_or(SceneError::BufferFull)?;
        let dst = self
            .buf
            .get_mut(self.len..end)
            .ok_or(SceneError::BufferFull)?;
        dst.copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }

    fn cmd(&mut self, op: u8, payload: &[&[u8]]) -> Result<(), SceneError> {
        let total: usize = payload.iter().map(|p| p.len()).sum();
        let plen = u16::try_from(total).map_err(|_| SceneError::TooManyPoints)?;
        let rollback = self.len;
        let result = self.cmd_body(op, plen, payload);
        if result.is_err() {
            self.len = rollback;
        }
        result
    }

    fn cmd_body(&mut self, op: u8, plen: u16, payload: &[&[u8]]) -> Result<(), SceneError> {
        self.put(&[op])?;
        self.put(&plen.to_le_bytes())?;
        for part in payload {
            self.put(part)?;
        }
        Ok(())
    }

    /// Encodes a command whose payload is `head` then up to eight `f32`s.
    fn cmd_f32(&mut self, op: u8, head: &[u8], vals: &[f32]) -> Result<(), SceneError> {
        let mut bytes = [0u8; 32];
        if vals.len() > 8 {
            return Err(SceneError::TooManyPoints);
        }
        for (i, v) in vals.iter().enumerate() {
            let at = i * 4;
            if let Some(slot) = bytes.get_mut(at..at + 4) {
                slot.copy_from_slice(&v.to_le_bytes());
            }
        }
        let tail = bytes.get(..vals.len() * 4).unwrap_or(&[]);
        self.cmd(op, &[head, tail])
    }

    /// Pushes the current transform + clip + paint state.
    pub fn save(&mut self) -> Result<(), SceneError> {
        self.cmd(opcode::SAVE, &[])
    }

    /// Pops to the most recently saved state.
    pub fn restore(&mut self) -> Result<(), SceneError> {
        self.cmd(opcode::RESTORE, &[])
    }

    /// Translates the current transform.
    pub fn translate(&mut self, x: f32, y: f32) -> Result<(), SceneError> {
        self.cmd_f32(opcode::TRANSLATE, &[], &[x, y])
    }

    /// Rotates the current transform; positive is clockwise (y-down).
    pub fn rotate(&mut self, radians: f32) -> Result<(), SceneError> {
        self.cmd_f32(opcode::ROTATE, &[], &[radians])
    }

    /// Sets the fill color.
    pub fn fill_color(&mut self, color: Rgba8) -> Result<(), SceneError> {
        self.cmd(opcode::FILL_COLOR, &[&color.to_u32().to_le_bytes()])
    }

    /// Sets the stroke color and width.
    pub fn stroke(&mut self, color: Rgba8, width: f32) -> Result<(), SceneError> {
        self.cmd(
            opcode::STROKE,
            &[&color.to_u32().to_le_bytes(), &width.to_le_bytes()],
        )
    }

    /// A stroked line segment.
    pub fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32) -> Result<(), SceneError> {
        self.cmd_f32(opcode::LINE, &[], &[x1, y1, x2, y2])
    }

    /// An open stroked polyline.
    pub fn polyline(&mut self, points: &[[f32; 2]]) -> Result<(), SceneError> {
        self.points_cmd(opcode::POLYLINE, &[], points)
    }

    /// A closed polygon, filled and/or stroked per `mode`.
    pub fn polygon(&mut self, mode: PaintMode, points: &[[f32; 2]]) -> Result<(), SceneError> {
        self.points_cmd(opcode::POLYGON, &[mode.to_u8()], points)
    }

    fn points_cmd(&mut self, op: u8, head: &[u8], points: &[[f32; 2]]) -> Result<(), SceneError> {
        let byte_len = points
            .len()
            .checked_mul(8)
            .ok_or(SceneError::TooManyPoints)?;
        let total = byte_len
            .checked_add(head.len())
            .ok_or(SceneError::TooManyPoints)?;
        let plen = u16::try_from(total).map_err(|_| SceneError::TooManyPoints)?;
        let rollback = self.len;
        let result = self.points_body(op, plen, head, points);
        if result.is_err() {
            self.len = rollback;
        }
        result
    }

    fn points_body(
        &mut self,
        op: u8,
        plen: u16,
        head: &[u8],
        points: &[[f32; 2]],
    ) -> Result<(), SceneError> {
        self.put(&[op])?;
        self.put(&plen.to_le_bytes())?;
        self.put(head)?;
        for p in points {
            self.put(&p[0].to_le_bytes())?;
            self.put(&p[1].to_le_bytes())?;
        }
        Ok(())
    }

    /// An axis-aligned rectangle.
    pub fn rect(
        &mut self,
        mode: PaintMode,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) -> Result<(), SceneError> {
        self.cmd_f32(opcode::RECT, &[mode.to_u8()], &[x, y, w, h])
    }

    /// A circle.
    pub fn circle(&mut self, mode: PaintMode, cx: f32, cy: f32, r: f32) -> Result<(), SceneError> {
        self.cmd_f32(opcode::CIRCLE, &[mode.to_u8()], &[cx, cy, r])
    }

    /// A stroked circular arc; `start` 0 is +x, positive sweeps clockwise
    /// (y-down).
    pub fn arc(
        &mut self,
        cx: f32,
        cy: f32,
        r: f32,
        start: f32,
        sweep: f32,
    ) -> Result<(), SceneError> {
        self.cmd_f32(opcode::ARC, &[], &[cx, cy, r, start, sweep])
    }

    /// A text run drawn with the current fill color.
    pub fn text(
        &mut self,
        x: f32,
        y: f32,
        size: f32,
        anchor: Anchor,
        text: &str,
    ) -> Result<(), SceneError> {
        if text.len() > MAX_TEXT_BYTES {
            return Err(SceneError::TextTooLong);
        }
        let mut head = [0u8; 13];
        head[0..4].copy_from_slice(&size.to_le_bytes());
        head[4] = anchor.to_u8();
        head[5..9].copy_from_slice(&x.to_le_bytes());
        head[9..13].copy_from_slice(&y.to_le_bytes());
        self.cmd(opcode::TEXT, &[&head, text.as_bytes()])
    }

    /// Intersects the current clip with a rectangle.
    pub fn clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) -> Result<(), SceneError> {
        self.cmd_f32(opcode::CLIP_RECT, &[], &[x, y, w, h])
    }
}

#[cfg(test)]
mod tests;
