//! Bounded validation for source contracts and portable captures.

use crate::{
    CHARACTERIZATION_CAPTURE_SCHEMA_VERSION, CaptureSegmentKind, CharacterizationCapture,
    DeadzoneEvidenceMethod as Method, DeadzoneEvidenceStatus as Status, DeviceProfile,
    MAX_CHARACTERIZATION_CAPTURE_SAMPLES, MAX_CHARACTERIZATION_PRODUCT_NAME_BYTES,
    RawReportDecoder, SOURCE_AXIS_CONTRACT_SCHEMA_VERSION, SamplingSource, SourceAxisContract,
    SourceAxisRange, TimestampSource, TimingObservation,
};

use super::AnalysisError;
use super::statistics::sample_time;

const MAX_CAPTURE_AXES: usize = 64;
const MAX_CAPTURE_SEGMENTS: usize = 65;
const MAX_INSTANCE_ID_BYTES: usize = 256;
const MAX_LOGICAL_NAME_BYTES: usize = 64;
const MAX_REPORT_HEX_BYTES: usize = 8_192;

pub(super) fn validate_source_contract(
    contract: &SourceAxisContract,
    contract_digest: &str,
    capture: &CharacterizationCapture,
    profile: &DeviceProfile,
) -> Result<Option<RawReportDecoder>, AnalysisError> {
    if contract.schema_version != SOURCE_AXIS_CONTRACT_SCHEMA_VERSION {
        return contract_mismatch("unsupported source-axis contract schema");
    }
    if contract_digest != capture.source_contract_digest {
        return Err(AnalysisError::ContractDigestMismatch {
            actual: contract_digest.to_owned(),
            expected: capture.source_contract_digest.clone(),
        });
    }
    let contract_identity = (contract.device.vendor_id, contract.device.product_id);
    let capture_identity = (capture.device.vendor_id, capture.device.product_id);
    let profile_identity = (profile.device.vendor_id, profile.device.product_id);
    if contract_identity == (0, 0) {
        return contract_mismatch("characterization requires a specific device identity");
    }
    if contract_identity != capture_identity || contract_identity != profile_identity {
        return contract_mismatch("contract, capture, and profile device identities differ");
    }
    if !source_ranges_equal_exact(&contract.axes, &capture.source_axes) {
        return contract_mismatch("capture axes do not match the exact source-axis contract");
    }
    if !valid_product_name(&contract.device.product)
        || !valid_product_name(&capture.device.product)
        || !valid_product_name(&profile.device.product)
    {
        return contract_mismatch("a device product name is outside its byte limit");
    }
    match capture.source {
        SamplingSource::AppleHid | SamplingSource::NativeHid => RawReportDecoder::new(contract)
            .map(Some)
            .map_err(|source| invalid_error(&format!("raw report layout is invalid: {source}"))),
        SamplingSource::BrowserGamepad | SamplingSource::Synthetic => {
            if contract.raw_report_layout.is_some() {
                contract_mismatch("a non-native source contract has a raw report layout")
            } else {
                Ok(None)
            }
        }
    }
}

pub(super) fn validate_capture(
    capture: &CharacterizationCapture,
    profile: &DeviceProfile,
    raw_report_decoder: Option<&RawReportDecoder>,
) -> Result<(), AnalysisError> {
    if capture.schema_version != CHARACTERIZATION_CAPTURE_SCHEMA_VERSION {
        return invalid("unsupported capture schema");
    }
    if capture.device.vendor_id != profile.device.vendor_id
        || capture.device.product_id != profile.device.product_id
    {
        return invalid("capture and baseline profile device identities differ");
    }
    if capture.device_instance_id.is_empty()
        || capture.device_instance_id.len() > MAX_INSTANCE_ID_BYTES
    {
        return invalid("the capture device instance ID is invalid");
    }
    validate_observation(capture)?;
    let axis_count = capture
        .samples
        .first()
        .map(|sample| sample.axes.len())
        .ok_or_else(|| invalid_error("the capture has no samples"))?;
    if axis_count == 0 || axis_count > MAX_CAPTURE_AXES {
        return invalid("the capture axis count is outside its limit");
    }
    if capture.samples.len() > MAX_CHARACTERIZATION_CAPTURE_SAMPLES {
        return invalid("the capture sample count is outside its limit");
    }
    validate_source_ranges(capture, axis_count)?;
    validate_samples(capture, axis_count, raw_report_decoder)?;
    validate_segments(capture)?;
    validate_deadzone_evidence(capture)
}

fn validate_observation(capture: &CharacterizationCapture) -> Result<(), AnalysisError> {
    let valid = match capture.source {
        SamplingSource::BrowserGamepad => {
            capture.timing_observation == TimingObservation::PolledStateUpdates
        }
        SamplingSource::AppleHid | SamplingSource::NativeHid => {
            capture.timing_observation == TimingObservation::ReportCallbacks
        }
        SamplingSource::Synthetic => {
            capture.timing_observation == TimingObservation::InjectedSamples
        }
    };
    if valid {
        Ok(())
    } else {
        invalid("the timing observation does not match the sampling source")
    }
}

fn validate_source_ranges(
    capture: &CharacterizationCapture,
    axis_count: usize,
) -> Result<(), AnalysisError> {
    if capture.source_axes.len() != axis_count {
        return invalid("the trusted source ranges do not cover all axes");
    }
    for (index, range) in capture.source_axes.iter().enumerate() {
        if range.source_index != index
            || !range.minimum.is_finite()
            || !range.maximum.is_finite()
            || range.minimum >= range.maximum
        {
            return invalid("a trusted source range is invalid");
        }
    }
    Ok(())
}

fn validate_samples(
    capture: &CharacterizationCapture,
    axis_count: usize,
    raw_report_decoder: Option<&RawReportDecoder>,
) -> Result<(), AnalysisError> {
    let raw_report_source = matches!(
        capture.source,
        SamplingSource::AppleHid | SamplingSource::NativeHid
    );
    for sample in &capture.samples {
        let invalid_axes = sample.axes.len() != axis_count
            || sample.axes.iter().enumerate().any(|(index, value)| {
                let range = capture.source_axes[index];
                !value.is_finite() || *value < range.minimum || *value > range.maximum
            });
        if invalid_axes {
            return invalid("a sample is outside the bounded source contract");
        }
        validate_sample_report(sample, raw_report_source, raw_report_decoder)?;
        if capture.timestamp_source == TimestampSource::Source && sample.source_at_us.is_none() {
            return invalid("the selected source clock is missing from a sample");
        }
    }
    for pair in capture.samples.windows(2) {
        if pair[1].sequence != pair[0].sequence.wrapping_add(1)
            || sample_time(&pair[1], capture.timestamp_source)
                <= sample_time(&pair[0], capture.timestamp_source)
        {
            return invalid("sample sequences and selected timestamps must increase");
        }
    }
    Ok(())
}

fn validate_sample_report(
    sample: &crate::CaptureSample,
    raw_report_source: bool,
    raw_report_decoder: Option<&RawReportDecoder>,
) -> Result<(), AnalysisError> {
    let Some(report_hex) = &sample.report_hex else {
        return if raw_report_source {
            invalid("a native sample has no raw report")
        } else {
            Ok(())
        };
    };
    if !raw_report_source {
        return invalid("a non-native sample contains a raw report");
    }
    let report = decode_report_hex(report_hex)
        .ok_or_else(|| invalid_error("a raw report does not use canonical hexadecimal form"))?;
    let decoder = raw_report_decoder
        .ok_or_else(|| invalid_error("a native sample has no raw report decoder"))?;
    let decoded = decoder.decode(&report).map_err(|source| {
        invalid_error(&format!(
            "raw report {} does not match its decoder: {source}",
            sample.sequence
        ))
    })?;
    if decoded
        .iter()
        .zip(&sample.axes)
        .any(|(raw, recorded)| raw.to_bits() != recorded.to_bits())
    {
        return invalid("a decoded raw report does not match its recorded axes");
    }
    Ok(())
}

fn decode_report_hex(report: &str) -> Option<Vec<u8>> {
    let bytes = report.as_bytes();
    let valid = !bytes.is_empty()
        && bytes.len() <= MAX_REPORT_HEX_BYTES
        && bytes.len() % 3 == 2
        && bytes.iter().enumerate().all(|(index, byte)| {
            if index % 3 == 2 {
                *byte == b' '
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)
            }
        });
    if !valid {
        return None;
    }
    Some(
        bytes
            .chunks(3)
            .map(|pair| (hex_nibble(pair[0]) << 4) | hex_nibble(pair[1]))
            .collect(),
    )
}

fn hex_nibble(byte: u8) -> u8 {
    if byte.is_ascii_digit() {
        byte - b'0'
    } else {
        byte - b'a' + 10
    }
}

fn validate_segments(capture: &CharacterizationCapture) -> Result<(), AnalysisError> {
    if capture.segments.is_empty() || capture.segments.len() > MAX_CAPTURE_SEGMENTS {
        return invalid("the capture segment count is outside its limit");
    }
    let first = capture.samples.first().map(|sample| sample.sequence);
    let last = capture.samples.last().map(|sample| sample.sequence);
    for segment in &capture.segments {
        let invalid_name = match &segment.action {
            CaptureSegmentKind::Movement { logical, .. } => {
                logical.is_empty() || logical.len() > MAX_LOGICAL_NAME_BYTES
            }
            CaptureSegmentKind::Idle => false,
        };
        if invalid_name
            || segment.start_sequence > segment.end_sequence
            || first.is_none_or(|value| segment.start_sequence < value)
            || last.is_none_or(|value| segment.end_sequence > value)
        {
            return invalid("a capture segment is invalid");
        }
    }
    if first
        != capture
            .segments
            .first()
            .map(|segment| segment.start_sequence)
        || last != capture.segments.last().map(|segment| segment.end_sequence)
        || capture
            .segments
            .windows(2)
            .any(|pair| pair[1].start_sequence != pair[0].end_sequence.wrapping_add(1))
    {
        return invalid("capture segments do not partition the samples");
    }
    Ok(())
}

fn validate_deadzone_evidence(capture: &CharacterizationCapture) -> Result<(), AnalysisError> {
    let evidence = &capture.deadzone_evidence;
    let raw_source = matches!(
        capture.source,
        SamplingSource::AppleHid | SamplingSource::NativeHid
    );
    let raw_report_count = u64::try_from(
        capture
            .samples
            .iter()
            .filter(|sample| sample.report_hex.is_some())
            .count(),
    )
    .unwrap_or(u64::MAX);
    let valid = match (evidence.status, evidence.method, evidence.sample_count) {
        (Status::Unknown, Method::Unmeasured, 0) => true,
        (Status::NotObserved, Method::RawHidReports, count) => {
            raw_source && count > 0 && count == raw_report_count
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        invalid("platform dead-zone evidence is inconsistent")
    }
}

fn source_ranges_equal_exact(left: &[SourceAxisRange], right: &[SourceAxisRange]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.source_index == right.source_index
                && left.minimum.to_bits() == right.minimum.to_bits()
                && left.maximum.to_bits() == right.maximum.to_bits()
                && left.neutral_position == right.neutral_position
        })
}

fn valid_product_name(product: &Option<String>) -> bool {
    product.as_ref().is_none_or(|value| {
        !value.is_empty() && value.len() <= MAX_CHARACTERIZATION_PRODUCT_NAME_BYTES
    })
}

fn contract_mismatch<T>(detail: &str) -> Result<T, AnalysisError> {
    Err(AnalysisError::ContractMismatch {
        detail: detail.to_owned(),
    })
}

fn invalid<T>(detail: &str) -> Result<T, AnalysisError> {
    Err(invalid_error(detail))
}

fn invalid_error(detail: &str) -> AnalysisError {
    AnalysisError::InvalidCapture {
        detail: detail.to_owned(),
    }
}
