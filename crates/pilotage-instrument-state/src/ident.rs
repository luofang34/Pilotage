//! Bounded waypoint/fix identifiers for display (ADR-0031 idents).
//!
//! An ident is at most [`IdentStr::CAPACITY`] bytes from the closed
//! charset `A–Z`, `0–9`, space, and `-` — the vocabulary an HSI readout
//! can commit to covering. Construction validates; the wire decoder maps
//! anything malformed to the [`IdentStr::INVALID`] sentinel, which
//! [`crate::validate_state`] flags so the nav group fails rather than
//! displaying text nobody vetted.

/// Why a string cannot become an [`IdentStr`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IdentError {
    /// Longer than [`IdentStr::CAPACITY`] bytes.
    #[error("ident exceeds {} bytes", IdentStr::CAPACITY)]
    TooLong,
    /// A byte outside `A–Z`, `0–9`, space, `-`.
    #[error("ident byte {byte:#04x} outside the ident charset")]
    Charset {
        /// The offending byte.
        byte: u8,
    },
}

const fn ident_byte_ok(byte: u8) -> bool {
    byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b' ' || byte == b'-'
}

/// A validated, fixed-capacity ident. Empty means "no ident" and renders
/// as dashes; [`Self::INVALID`] marks malformed wire content and fails
/// the nav group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdentStr {
    len: u8,
    bytes: [u8; Self::CAPACITY],
}

impl IdentStr {
    /// Maximum ident length in bytes.
    pub const CAPACITY: usize = 8;

    /// The wire length marker for malformed content.
    const INVALID_LEN: u8 = 0xFF;

    /// No ident.
    pub const EMPTY: IdentStr = IdentStr {
        len: 0,
        bytes: [0; Self::CAPACITY],
    };

    /// Malformed wire content: never constructible from a string, only
    /// decoded, and flagged by validation.
    pub const INVALID: IdentStr = IdentStr {
        len: Self::INVALID_LEN,
        bytes: [0; Self::CAPACITY],
    };

    /// Validates and stores `ident`.
    pub const fn new(ident: &str) -> Result<IdentStr, IdentError> {
        let raw = ident.as_bytes();
        if raw.len() > Self::CAPACITY {
            return Err(IdentError::TooLong);
        }
        let mut bytes = [0u8; Self::CAPACITY];
        let mut i = 0;
        while i < raw.len() {
            if !ident_byte_ok(raw[i]) {
                return Err(IdentError::Charset { byte: raw[i] });
            }
            bytes[i] = raw[i];
            i += 1;
        }
        Ok(IdentStr {
            len: raw.len() as u8,
            bytes,
        })
    }

    /// The ident text; empty for [`Self::EMPTY`] and [`Self::INVALID`].
    pub fn as_str(&self) -> &str {
        let len = if self.len == Self::INVALID_LEN {
            0
        } else {
            self.len as usize
        };
        let bytes = self.bytes.get(..len).unwrap_or(&[]);
        core::str::from_utf8(bytes).unwrap_or("")
    }

    /// No ident is present.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The wire carried malformed content for this ident.
    pub const fn is_invalid(&self) -> bool {
        self.len == Self::INVALID_LEN
    }

    /// The canonical 9-byte wire atom: length then zero-padded bytes.
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
    pub(crate) fn from_wire(wire: &[u8; Self::CAPACITY + 1]) -> IdentStr {
        let len = wire[0];
        if len == Self::INVALID_LEN {
            return Self::INVALID;
        }
        if len as usize > Self::CAPACITY {
            return Self::INVALID;
        }
        let mut bytes = [0u8; Self::CAPACITY];
        let mut i = 0;
        while i < Self::CAPACITY {
            let byte = wire[i + 1];
            if i < len as usize {
                if !ident_byte_ok(byte) {
                    return Self::INVALID;
                }
                bytes[i] = byte;
            } else if byte != 0 {
                return Self::INVALID;
            }
            i += 1;
        }
        IdentStr { len, bytes }
    }
}

impl Default for IdentStr {
    fn default() -> Self {
        Self::EMPTY
    }
}

#[cfg(test)]
mod tests;
