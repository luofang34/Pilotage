//! Declarative decoding for raw HID characterization reports.

use super::capture::{
    MAX_CHARACTERIZATION_RAW_REPORT_BYTES, RawReportAxisField, SourceAxisContract,
};

const MAX_EXACT_INTEGER_BITS: u8 = 24;

/// Errors from a source-axis contract's raw report decoder.
#[derive(Debug, thiserror::Error)]
pub enum RawReportError {
    /// The source-axis contract has no raw report layout.
    #[error("the source-axis contract has no raw report layout")]
    MissingLayout,
    /// The declared report size is outside the shared limit.
    #[error("raw report byte count {actual} is outside 1..={maximum}")]
    InvalidReportByteCount {
        /// The declared report byte count.
        actual: usize,
        /// The maximum accepted report byte count.
        maximum: usize,
    },
    /// The layout does not define exactly one field for each source axis.
    #[error("raw report field count {fields} does not match source axis count {axes}")]
    AxisCountMismatch {
        /// The raw field count.
        fields: usize,
        /// The source axis count.
        axes: usize,
    },
    /// A raw field is not in source-index order.
    #[error("raw report field {position} targets source index {source_index}")]
    AxisIndexMismatch {
        /// The field position.
        position: usize,
        /// The declared source index.
        source_index: usize,
    },
    /// A raw field is outside the report or the exact integer range.
    #[error(
        "raw report field {source_index} at bit {bit_offset} with width {bit_width} is invalid"
    )]
    InvalidAxisField {
        /// The field's source axis.
        source_index: usize,
        /// The first report bit.
        bit_offset: usize,
        /// The field width.
        bit_width: u8,
    },
    /// Two declared axes use the same report bit.
    #[error("raw report field for source index {source_index} overlaps another field")]
    OverlappingAxisField {
        /// The overlapping field's source axis.
        source_index: usize,
    },
    /// A source range cannot be represented exactly by its raw field.
    #[error("source range {minimum}..={maximum} for axis {source_index} is outside its raw field")]
    InvalidSourceRange {
        /// The source axis.
        source_index: usize,
        /// The declared minimum.
        minimum: f32,
        /// The declared maximum.
        maximum: f32,
    },
    /// A report does not have the contract's exact size.
    #[error("raw report has {actual} bytes; expected {expected}")]
    ReportLengthMismatch {
        /// The observed byte count.
        actual: usize,
        /// The required byte count.
        expected: usize,
    },
    /// A report does not have the contract's report ID.
    #[error("raw report ID {actual} does not match {expected}")]
    ReportIdMismatch {
        /// The observed report ID.
        actual: u8,
        /// The required report ID.
        expected: u8,
    },
}

/// A validated, reusable decoder for one exact source-axis contract.
#[derive(Debug, Clone)]
pub struct RawReportDecoder {
    report_byte_count: usize,
    report_id: Option<u8>,
    axes: Vec<RawReportAxisField>,
}

impl RawReportDecoder {
    /// Validates a source-axis contract and creates its raw report decoder.
    ///
    /// # Errors
    ///
    /// Returns [`RawReportError`] when the contract has no layout or the
    /// layout is incomplete, overlapping, or outside the report bounds.
    pub fn new(contract: &SourceAxisContract) -> Result<Self, RawReportError> {
        let layout = contract
            .raw_report_layout
            .as_ref()
            .ok_or(RawReportError::MissingLayout)?;
        validate_report_byte_count(layout.report_byte_count)?;
        if layout.axes.len() != contract.axes.len() {
            return Err(RawReportError::AxisCountMismatch {
                fields: layout.axes.len(),
                axes: contract.axes.len(),
            });
        }
        validate_axis_fields(layout.report_byte_count, layout.report_id, &layout.axes)?;
        validate_source_ranges(&layout.axes, contract)?;
        Ok(Self {
            report_byte_count: layout.report_byte_count,
            report_id: layout.report_id,
            axes: layout.axes.clone(),
        })
    }

    /// Returns the exact report byte count.
    #[must_use]
    pub const fn report_byte_count(&self) -> usize {
        self.report_byte_count
    }

    /// Decodes one exact raw report into source-axis values.
    ///
    /// # Errors
    ///
    /// Returns [`RawReportError`] when the report size or report ID differs
    /// from the validated contract.
    pub fn decode(&self, report: &[u8]) -> Result<Vec<f32>, RawReportError> {
        if report.len() != self.report_byte_count {
            return Err(RawReportError::ReportLengthMismatch {
                actual: report.len(),
                expected: self.report_byte_count,
            });
        }
        if let Some(expected) = self.report_id {
            let actual = report[0];
            if actual != expected {
                return Err(RawReportError::ReportIdMismatch { actual, expected });
            }
        }
        Ok(self
            .axes
            .iter()
            .map(|field| decode_field(report, *field))
            .collect())
    }
}

fn validate_source_ranges(
    fields: &[RawReportAxisField],
    contract: &SourceAxisContract,
) -> Result<(), RawReportError> {
    for (field, range) in fields.iter().zip(&contract.axes) {
        let (encoded_minimum, encoded_maximum) = integer_limits(*field);
        let valid = range.source_index == field.source_index
            && range.minimum.is_finite()
            && range.maximum.is_finite()
            && range.minimum < range.maximum
            && range.minimum.fract() == 0.0
            && range.maximum.fract() == 0.0
            && f64::from(range.minimum) >= encoded_minimum as f64
            && f64::from(range.maximum) <= encoded_maximum as f64;
        if !valid {
            return Err(RawReportError::InvalidSourceRange {
                source_index: field.source_index,
                minimum: range.minimum,
                maximum: range.maximum,
            });
        }
    }
    Ok(())
}

fn integer_limits(field: RawReportAxisField) -> (i64, i64) {
    if field.signed {
        let magnitude = 1i64 << (field.bit_width - 1);
        (-magnitude, magnitude - 1)
    } else {
        (0, (1i64 << field.bit_width) - 1)
    }
}

fn validate_report_byte_count(actual: usize) -> Result<(), RawReportError> {
    if actual == 0 || actual > MAX_CHARACTERIZATION_RAW_REPORT_BYTES {
        Err(RawReportError::InvalidReportByteCount {
            actual,
            maximum: MAX_CHARACTERIZATION_RAW_REPORT_BYTES,
        })
    } else {
        Ok(())
    }
}

fn validate_axis_fields(
    report_byte_count: usize,
    report_id: Option<u8>,
    axes: &[RawReportAxisField],
) -> Result<(), RawReportError> {
    let report_bits = report_byte_count.saturating_mul(8);
    let mut used = vec![false; report_bits];
    if report_id.is_some() {
        used[..8].fill(true);
    }
    for (position, field) in axes.iter().enumerate() {
        if field.source_index != position {
            return Err(RawReportError::AxisIndexMismatch {
                position,
                source_index: field.source_index,
            });
        }
        let width = usize::from(field.bit_width);
        let end = field.bit_offset.checked_add(width);
        if field.bit_width == 0
            || field.bit_width > MAX_EXACT_INTEGER_BITS
            || end.is_none_or(|value| value > report_bits)
        {
            return Err(RawReportError::InvalidAxisField {
                source_index: field.source_index,
                bit_offset: field.bit_offset,
                bit_width: field.bit_width,
            });
        }
        let range = field.bit_offset..end.unwrap_or(field.bit_offset);
        if used[range.clone()].iter().any(|value| *value) {
            return Err(RawReportError::OverlappingAxisField {
                source_index: field.source_index,
            });
        }
        used[range].fill(true);
    }
    Ok(())
}

fn decode_field(report: &[u8], field: RawReportAxisField) -> f32 {
    let mut value = 0u32;
    for output_bit in 0..field.bit_width {
        let report_bit = field.bit_offset + usize::from(output_bit);
        let bit = (report[report_bit / 8] >> (report_bit % 8)) & 1;
        value |= u32::from(bit) << output_bit;
    }
    if field.signed {
        let sign_bit = 1u32 << (field.bit_width - 1);
        if value & sign_bit != 0 {
            return ((i64::from(value)) - (1i64 << field.bit_width)) as f32;
        }
    }
    value as f32
}

#[cfg(test)]
mod tests;
