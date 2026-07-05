//! Raw HID input-report decoding for the RadioMaster Pocket.
//!
//! Only [`le_u16_words`] is a twin of `tools/hid-probe/src/decode.rs`: the
//! same generic little-endian word-splitting, duplicated rather than shared
//! because it is a few lines of pure reshaping with no behavior either binary
//! would want to change independently of the other. `hid-probe`'s decoder
//! deliberately assumes no report layout (it exists to produce the raw hex
//! dump a layout is discovered from); this module's [`decode_report`] and
//! [`report_button_mask`] are not mirrored there — they encode the
//! RadioMaster Pocket's specific report layout empirically verified against
//! `crates/pilotage-input/registry/fixtures/radiomaster-pocket-capture.json`:
//! 19 bytes total, the first 3 bytes a button bitmap, the remaining 16 bytes
//! 8 little-endian `u16` axis words. If this grows further decode logic
//! (calibration, axis semantics), move it into `pilotage-input` instead of
//! adding a third copy.

/// Decodes `bytes` as a sequence of little-endian `u16` words, dropping a
/// trailing odd byte if present.
#[must_use]
pub fn le_u16_words(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect()
}

/// Converts a RadioMaster Pocket input report into a
/// [`pilotage_input::RawDeviceSample`]: the first 3 bytes are a 24-button
/// bitmap, the remaining bytes are 8
/// little-endian 11-bit-range `u16` axis words, per the layout recorded in
/// `crates/pilotage-input/registry/radiomaster-pocket.json`.
///
/// `sampled_at` is supplied by the caller (this module is pure decode, not a
/// clock source).
#[must_use]
pub fn decode_report(
    bytes: &[u8],
    sampled_at: pilotage_timing::MonoTimestamp,
) -> pilotage_input::RawDeviceSample {
    let buttons = report_button_mask(bytes);
    let axis_bytes = bytes.get(3..).unwrap_or(&[]);
    let axes = le_u16_words(axis_bytes)
        .into_iter()
        .map(f32::from)
        .collect();
    pilotage_input::RawDeviceSample::new(axes, buttons, sampled_at)
}

/// Packs the first 3 report bytes (24 buttons) into a `u64` bitmask,
/// little-endian byte order matching the report layout.
fn report_button_mask(bytes: &[u8]) -> u64 {
    let mut mask = 0u64;
    for (index, byte) in bytes.iter().take(3).enumerate() {
        mask |= u64::from(*byte) << (index * 8);
    }
    mask
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{decode_report, le_u16_words, report_button_mask};
    use pilotage_timing::MonoTimestamp;

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

    #[test]
    fn button_mask_packs_three_bytes_little_endian() {
        assert_eq!(
            report_button_mask(&[0b0000_0001, 0b0000_0000, 0b0000_0000]),
            1
        );
        assert_eq!(report_button_mask(&[0, 0b0000_0001, 0]), 1 << 8);
    }

    #[test]
    fn decode_report_splits_buttons_and_axes() {
        let mut report = vec![0b0000_0001u8, 0, 0];
        report.extend_from_slice(&1024u16.to_le_bytes());
        report.extend_from_slice(&0u16.to_le_bytes());
        let sample = decode_report(&report, MonoTimestamp::from_nanos(0));
        assert!(sample.button_held(0));
        assert!(!sample.button_held(1));
        assert_eq!(sample.axes, vec![1024.0, 0.0]);
    }
}
