//! Target device identity for the `read`/`capture` subcommands.

/// RadioMaster Pocket USB vendor ID (verified against the physical unit
/// connected during development; see task record for the enumeration
/// output).
pub const TARGET_VENDOR_ID: u16 = 0x1209;
/// RadioMaster Pocket USB product ID.
pub const TARGET_PRODUCT_ID: u16 = 0x4F54;

/// Number of button bytes before the RadioMaster axis fields.
pub const BUTTON_BYTE_COUNT: usize = 3;
/// Number of 16-bit RadioMaster axis fields.
pub const AXIS_COUNT: usize = 8;
/// Total bytes in one RadioMaster input report.
pub const REPORT_LEN: usize = BUTTON_BYTE_COUNT + AXIS_COUNT * 2;

/// Decodes the RadioMaster axis fields from one complete report.
///
/// # Errors
///
/// Returns the observed length when the report does not have the required
/// RadioMaster layout.
pub fn decode_axes(report: &[u8]) -> Result<Vec<f32>, usize> {
    if report.len() != REPORT_LEN {
        return Err(report.len());
    }
    Ok(report[BUTTON_BYTE_COUNT..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| f32::from(u16::from_le_bytes(*pair)))
        .collect())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{REPORT_LEN, decode_axes};

    #[test]
    fn decodes_the_radio_axis_words_after_buttons() {
        let report = [0, 0, 0, 0, 4, 0, 4, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            decode_axes(&report).expect("decode"),
            [1024.0, 1024.0, 0.0, 1024.0, 0.0, 0.0, 0.0, 0.0]
        );
    }

    #[test]
    fn rejects_an_unexpected_report_length() {
        assert_eq!(decode_axes(&[0; REPORT_LEN - 1]), Err(REPORT_LEN - 1));
    }
}
