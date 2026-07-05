//! Turns a raw HID input-report byte buffer into a hex string and a decoded
//! little-endian `u16` word view, for human inspection.
//!
//! This is deliberately dumb: it does not assume any particular axis layout,
//! bit width, or button bitmap position. Those assumptions belong in a
//! device profile (`crates/pilotage-input/registry/`), decided only after
//! looking at this raw output.

/// Formats `bytes` as lowercase hex pairs separated by single spaces, e.g.
/// `"01 ff 00"`.
#[must_use]
pub fn to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Decodes `bytes` as a sequence of little-endian `u16` words, dropping a
/// trailing odd byte if present (reported so callers can see it happened).
#[must_use]
pub fn le_u16_words(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{le_u16_words, to_hex};

    #[test]
    fn hex_formats_bytes_lowercase_space_separated() {
        assert_eq!(to_hex(&[0x01, 0xff, 0x00]), "01 ff 00");
    }

    #[test]
    fn hex_of_empty_is_empty_string() {
        assert_eq!(to_hex(&[]), "");
    }

    #[test]
    fn le_words_decodes_pairs_little_endian() {
        assert_eq!(
            le_u16_words(&[0xff, 0x07, 0x00, 0x08]),
            vec![0x07ff, 0x0800]
        );
    }

    #[test]
    fn le_words_drops_trailing_odd_byte() {
        assert_eq!(le_u16_words(&[0x01, 0x00, 0x02]), vec![0x0001]);
    }
}
