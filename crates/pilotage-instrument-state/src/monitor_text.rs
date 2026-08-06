//! Machine-monitoring text channel (AIR-IN-014): bounded live text a
//! non-flight source publishes for display.
//!
//! The channel is advisory machine text — build stamps, link health,
//! payload status — not flight data: it carries its own slow freshness
//! policy so an irregular feed does not flap, and a `revision` counter
//! so consumers can detect re-published identical content without
//! diffing. Content is bounded ([`MonitorText::MAX_LINES`] lines of
//! [`TextLine::CAPACITY`] bytes) from a closed charset, and malformed
//! wire content decodes to the [`TextLine::INVALID`] sentinel that
//! [`crate::validate_state`] flags — text nobody vetted never displays.

/// Why a string cannot become a [`TextLine`], or lines cannot become a
/// [`MonitorText`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TextError {
    /// Longer than [`TextLine::CAPACITY`] bytes.
    #[error("text line exceeds {} bytes", TextLine::CAPACITY)]
    TooLong,
    /// A byte outside `A–Z`, `0–9`, space, `-`, `.`.
    #[error("text byte {byte:#04x} outside the monitor charset")]
    Charset {
        /// The offending byte.
        byte: u8,
    },
    /// More than [`MonitorText::MAX_LINES`] lines.
    #[error("more than {} monitor lines", MonitorText::MAX_LINES)]
    TooManyLines,
}

const fn text_byte_ok(byte: u8) -> bool {
    byte.is_ascii_uppercase()
        || byte.is_ascii_digit()
        || byte == b' '
        || byte == b'-'
        || byte == b'.'
}

/// One validated, fixed-capacity monitor line. Empty renders nothing;
/// [`Self::INVALID`] marks malformed wire content and fails the group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextLine {
    len: u8,
    bytes: [u8; Self::CAPACITY],
}

impl TextLine {
    /// Maximum line length in bytes.
    pub const CAPACITY: usize = 32;

    /// The wire length marker for malformed content.
    const INVALID_LEN: u8 = 0xFF;

    /// No text.
    pub const EMPTY: TextLine = TextLine {
        len: 0,
        bytes: [0; Self::CAPACITY],
    };

    /// Malformed wire content: never constructible from a string, only
    /// decoded, and flagged by validation.
    pub const INVALID: TextLine = TextLine {
        len: Self::INVALID_LEN,
        bytes: [0; Self::CAPACITY],
    };

    /// Validates and stores `text`.
    pub const fn new(text: &str) -> Result<TextLine, TextError> {
        let raw = text.as_bytes();
        if raw.len() > Self::CAPACITY {
            return Err(TextError::TooLong);
        }
        let mut bytes = [0u8; Self::CAPACITY];
        let mut i = 0;
        while i < raw.len() {
            if !text_byte_ok(raw[i]) {
                return Err(TextError::Charset { byte: raw[i] });
            }
            bytes[i] = raw[i];
            i += 1;
        }
        Ok(TextLine {
            len: raw.len() as u8,
            bytes,
        })
    }

    /// The line text; empty for [`Self::EMPTY`] and [`Self::INVALID`].
    pub fn as_str(&self) -> &str {
        let len = if self.len == Self::INVALID_LEN {
            0
        } else {
            self.len as usize
        };
        let bytes = self.bytes.get(..len).unwrap_or(&[]);
        core::str::from_utf8(bytes).unwrap_or("")
    }

    /// No text is present.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The wire carried malformed content for this line.
    pub const fn is_invalid(&self) -> bool {
        self.len == Self::INVALID_LEN
    }

    /// The canonical 33-byte wire atom: length then zero-padded bytes.
    pub(crate) fn to_wire(self) -> [u8; Self::CAPACITY + 1] {
        let mut out = [0u8; Self::CAPACITY + 1];
        out[0] = self.len;
        let mut i = 0;
        while i < Self::CAPACITY {
            out[i + 1] = self.bytes[i];
            i += 1;
        }
        out
    }

    /// Decodes a wire atom. Anything malformed — over-length, an
    /// out-of-charset byte, or nonzero padding — becomes
    /// [`Self::INVALID`], never a partial string.
    pub(crate) fn from_wire(wire: &[u8; Self::CAPACITY + 1]) -> TextLine {
        let len = wire[0];
        if len == Self::INVALID_LEN || len as usize > Self::CAPACITY {
            return Self::INVALID;
        }
        let mut bytes = [0u8; Self::CAPACITY];
        let mut i = 0;
        while i < Self::CAPACITY {
            let byte = wire[i + 1];
            if i < len as usize {
                if !text_byte_ok(byte) {
                    return Self::INVALID;
                }
                bytes[i] = byte;
            } else if byte != 0 {
                return Self::INVALID;
            }
            i += 1;
        }
        TextLine { len, bytes }
    }
}

impl Default for TextLine {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// The live monitor readout: up to [`Self::MAX_LINES`] validated lines
/// and a wrapping content revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MonitorText {
    /// Wrapping counter a source advances on each accepted content
    /// change, so consumers detect re-published identical content
    /// without diffing.
    pub revision: u32,
    line_count: u8,
    lines: [TextLine; Self::MAX_LINES],
    /// The wire declared more lines than the channel can carry; set
    /// only by decoding, and flagged by validation.
    pub(crate) malformed: bool,
}

impl MonitorText {
    /// Maximum visible lines.
    pub const MAX_LINES: usize = 8;

    /// Validates and stores up to [`Self::MAX_LINES`] lines.
    pub fn new(revision: u32, lines: &[TextLine]) -> Result<MonitorText, TextError> {
        if lines.len() > Self::MAX_LINES {
            return Err(TextError::TooManyLines);
        }
        let mut stored = [TextLine::EMPTY; Self::MAX_LINES];
        for (slot, line) in stored.iter_mut().zip(lines) {
            *slot = *line;
        }
        Ok(MonitorText {
            revision,
            line_count: lines.len() as u8,
            lines: stored,
            malformed: false,
        })
    }

    /// Wire-side reconstruction; a count beyond capacity marks the
    /// whole channel malformed rather than truncating silently.
    pub(crate) fn from_wire(
        revision: u32,
        line_count: u8,
        lines: [TextLine; Self::MAX_LINES],
    ) -> Self {
        if line_count as usize > Self::MAX_LINES {
            return MonitorText {
                revision,
                line_count: 0,
                lines: [TextLine::EMPTY; Self::MAX_LINES],
                malformed: true,
            };
        }
        MonitorText {
            revision,
            line_count,
            lines,
            malformed: false,
        }
    }

    /// The visible lines.
    pub fn lines(&self) -> &[TextLine] {
        self.lines.get(..self.line_count as usize).unwrap_or(&[])
    }

    /// The wire carried content this channel cannot trust: an
    /// impossible line count or any invalid line.
    pub fn is_malformed(&self) -> bool {
        // All slots, not just visible lines: the wire decodes every
        // atom, and a malformed atom hiding past line_count must fail
        // the channel exactly like one in view.
        self.malformed || self.slots().iter().any(TextLine::is_invalid)
    }

    /// The stored line count (wire-side encoding).
    pub(crate) fn line_count(&self) -> u8 {
        self.line_count
    }

    /// Every stored line slot, used and unused (wire-side encoding).
    pub(crate) fn slots(&self) -> &[TextLine; Self::MAX_LINES] {
        &self.lines
    }
}

#[cfg(test)]
mod tests;
