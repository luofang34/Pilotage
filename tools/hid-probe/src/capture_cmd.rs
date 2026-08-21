//! Guided native HID capture for automatic characterization.

use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

use hidapi::{HidApi, HidDevice};
use pilotage_input::{
    CHARACTERIZATION_CAPTURE_SCHEMA_VERSION, CaptureSample, CaptureSegment, CaptureSegmentKind,
    CharacterizationCapture, DeadzoneEvidence, DeadzoneEvidenceMethod, DeadzoneEvidenceStatus,
    DeviceInfo, MAX_CHARACTERIZATION_CAPTURE_SAMPLES, MAX_CHARACTERIZATION_PRODUCT_NAME_BYTES,
    RawReportDecoder, SamplingSource, SourceAxisContract, TimestampSource, TimingObservation,
    content_digest,
};

use crate::artifact_file::{self, MAX_CAPTURE_BYTES};
use crate::decode::to_hex;
use crate::device::{TARGET_PRODUCT_ID, TARGET_VENDOR_ID};
use crate::error::ProbeError;
use crate::output::print_line;
use crate::read_cmd::REPORT_BUF_LEN;

const READ_TIMEOUT_MS: i32 = 200;
const SOURCE_CONTRACT_BYTES: &[u8] = include_bytes!(
    "../../../crates/pilotage-input/registry/radiomaster-pocket-source-contract.json"
);

struct Recorder {
    start: Instant,
    next_sequence: u64,
    samples: Vec<CaptureSample>,
    segments: Vec<CaptureSegment>,
    retained_sample_bytes: usize,
}

impl Recorder {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            next_sequence: 0,
            samples: Vec::new(),
            segments: Vec::new(),
            retained_sample_bytes: 0,
        }
    }

    fn record_segment(
        &mut self,
        device: &HidDevice,
        decoder: &RawReportDecoder,
        seconds: u64,
        action: CaptureSegmentKind,
    ) -> Result<(), ProbeError> {
        let first = self.next_sequence;
        let first_sample = self.samples.len();
        let deadline = Duration::from_secs(seconds);
        let phase_start = Instant::now();
        let mut report = [0u8; REPORT_BUF_LEN];
        while phase_start.elapsed() < deadline {
            let read = device.read_timeout(&mut report, READ_TIMEOUT_MS)?;
            if read == 0 {
                continue;
            }
            let axes = decoder
                .decode(&report[..read])
                .map_err(|source| ProbeError::RawReportDecode { source })?;
            self.push_sample(CaptureSample {
                sequence: self.next_sequence,
                observed_at_us: elapsed_micros(self.start),
                source_at_us: None,
                axes,
                report_hex: Some(to_hex(&report[..read])),
            })?;
        }
        self.finish_segment(first, first_sample, action)
    }

    fn finish_segment(
        &mut self,
        first_sequence: u64,
        first_sample_count: usize,
        action: CaptureSegmentKind,
    ) -> Result<(), ProbeError> {
        if self.samples.len() == first_sample_count {
            return Err(ProbeError::InvalidCapture {
                detail: "a guided segment received no HID report".to_owned(),
            });
        }
        self.segments.push(CaptureSegment {
            action,
            start_sequence: first_sequence,
            end_sequence: self.next_sequence.wrapping_sub(1),
        });
        Ok(())
    }

    fn push_sample(&mut self, sample: CaptureSample) -> Result<(), ProbeError> {
        self.push_sample_with_limits(
            sample,
            MAX_CHARACTERIZATION_CAPTURE_SAMPLES,
            MAX_CAPTURE_BYTES,
        )
    }

    fn push_sample_with_limits(
        &mut self,
        sample: CaptureSample,
        sample_limit: usize,
        memory_limit: usize,
    ) -> Result<(), ProbeError> {
        if self.samples.len() >= sample_limit {
            return Err(ProbeError::CaptureSampleLimit {
                limit: sample_limit,
            });
        }
        let retained = self
            .retained_sample_bytes
            .saturating_add(retained_sample_bytes(&sample));
        if retained > memory_limit {
            return Err(ProbeError::CaptureMemoryLimit {
                actual: retained,
                limit: memory_limit,
            });
        }
        self.samples.push(sample);
        self.retained_sample_bytes = retained;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        Ok(())
    }
}

/// Records an idle segment and one positive-first movement segment per axis.
///
/// # Errors
///
/// Returns [`ProbeError`] when the HID port, report decoder, serializer, or
/// output file fails.
pub fn run(
    idle_seconds: u64,
    movement_seconds: u64,
    logical_axes: &[String],
    out: &Path,
) -> Result<(), ProbeError> {
    let api = HidApi::new()?;
    let source_contract: SourceAxisContract = serde_json::from_slice(SOURCE_CONTRACT_BYTES)
        .map_err(|source| ProbeError::SourceContractParse { source })?;
    let decoder = RawReportDecoder::new(&source_contract)
        .map_err(|source| ProbeError::SourceContractLayout { source })?;
    let (device, device_instance_id, product) = open_target(&api)?;
    if product.as_ref().is_some_and(|value| {
        value.is_empty() || value.len() > MAX_CHARACTERIZATION_PRODUCT_NAME_BYTES
    }) {
        return Err(ProbeError::ProductNameLimit {
            limit: MAX_CHARACTERIZATION_PRODUCT_NAME_BYTES,
        });
    }
    let mut recorder = Recorder::new();
    print_line("Keep all controls at rest.");
    recorder.record_segment(&device, &decoder, idle_seconds, CaptureSegmentKind::Idle)?;
    for logical in logical_axes {
        print_line(&format!(
            "Move only {logical}. Move positive first. Use the full range. Return it to its normal neutral position."
        ));
        recorder.record_segment(
            &device,
            &decoder,
            movement_seconds,
            CaptureSegmentKind::Movement {
                logical: logical.clone(),
                positive_first: true,
            },
        )?;
    }
    let sample_count = u64::try_from(recorder.samples.len()).unwrap_or(u64::MAX);
    let capture = CharacterizationCapture {
        schema_version: CHARACTERIZATION_CAPTURE_SCHEMA_VERSION,
        device: DeviceInfo {
            vendor_id: TARGET_VENDOR_ID,
            product_id: TARGET_PRODUCT_ID,
            product,
        },
        device_instance_id,
        source: native_source(),
        timestamp_source: TimestampSource::Arrival,
        timing_observation: TimingObservation::ReportCallbacks,
        deadzone_evidence: DeadzoneEvidence {
            status: DeadzoneEvidenceStatus::NotObserved,
            method: DeadzoneEvidenceMethod::RawHidReports,
            sample_count,
        },
        source_contract_digest: digest_hex(SOURCE_CONTRACT_BYTES),
        source_axes: source_contract.axes,
        samples: recorder.samples,
        segments: recorder.segments,
    };
    write_capture(&capture, out)
}

fn open_target(api: &HidApi) -> Result<(HidDevice, String, Option<String>), ProbeError> {
    let targets: Vec<_> = api
        .device_list()
        .filter(|device| {
            device.vendor_id() == TARGET_VENDOR_ID && device.product_id() == TARGET_PRODUCT_ID
        })
        .collect();
    if targets.len() != 1 {
        return Err(ProbeError::TargetDeviceCount {
            found: targets.len(),
        });
    }
    let path = targets[0].path().to_owned();
    let product = targets[0].product_string().map(str::to_owned);
    let device = api.open_path(&path)?;
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| ProbeError::Clock { source })?
        .as_nanos();
    let token = format!("{}:{}:{epoch}", std::process::id(), path.to_string_lossy());
    Ok((device, digest_hex(token.as_bytes()), product))
}

fn native_source() -> SamplingSource {
    if cfg!(target_vendor = "apple") {
        SamplingSource::AppleHid
    } else {
        SamplingSource::NativeHid
    }
}

fn elapsed_micros(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX)
}

fn digest_hex(bytes: &[u8]) -> String {
    content_digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn write_capture(capture: &CharacterizationCapture, out: &Path) -> Result<(), ProbeError> {
    validate_capture_size(encoded_capture_size(capture)?)?;
    artifact_file::write_new_json(out, capture)
}

fn encoded_capture_size(capture: &CharacterizationCapture) -> Result<usize, ProbeError> {
    let mut counter = ByteCounter::default();
    serde_json::to_writer_pretty(&mut counter, capture)
        .map_err(|source| ProbeError::CaptureSerialize { source })?;
    Ok(counter.bytes.saturating_add(1))
}

fn validate_capture_size(actual: usize) -> Result<(), ProbeError> {
    if actual > MAX_CAPTURE_BYTES {
        Err(ProbeError::CaptureByteLimit {
            actual,
            limit: MAX_CAPTURE_BYTES,
        })
    } else {
        Ok(())
    }
}

fn retained_sample_bytes(sample: &CaptureSample) -> usize {
    std::mem::size_of::<CaptureSample>()
        .saturating_mul(2)
        .saturating_add(
            sample
                .axes
                .capacity()
                .saturating_mul(std::mem::size_of::<f32>()),
        )
        .saturating_add(sample.report_hex.as_ref().map_or(0, String::capacity))
}

#[derive(Default)]
struct ByteCounter {
    bytes: usize,
}

impl Write for ByteCounter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.bytes = self.bytes.saturating_add(buffer.len());
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::{MAX_CAPTURE_BYTES, Recorder, retained_sample_bytes, validate_capture_size};
    use pilotage_input::{CaptureSample, CaptureSegmentKind};

    #[test]
    fn a_later_empty_segment_is_rejected() {
        let mut recorder = Recorder::new();
        recorder.next_sequence = 8;
        recorder.samples = (0..8)
            .map(|sequence| CaptureSample {
                sequence,
                observed_at_us: sequence,
                source_at_us: None,
                axes: vec![0.0],
                report_hex: None,
            })
            .collect();
        assert!(
            recorder
                .finish_segment(8, 8, CaptureSegmentKind::Idle)
                .is_err()
        );
        assert!(recorder.segments.is_empty());
    }

    #[test]
    fn recorder_and_encoder_enforce_the_shared_capture_limits() {
        let mut recorder = Recorder::new();
        let sample = CaptureSample {
            sequence: 0,
            observed_at_us: 0,
            source_at_us: None,
            axes: vec![0.0],
            report_hex: None,
        };
        assert!(
            recorder
                .push_sample_with_limits(sample.clone(), 1, MAX_CAPTURE_BYTES)
                .is_ok()
        );
        assert!(
            recorder
                .push_sample_with_limits(sample.clone(), 1, MAX_CAPTURE_BYTES)
                .is_err()
        );
        let retained = retained_sample_bytes(&sample);
        let mut recorder = Recorder::new();
        assert!(
            recorder
                .push_sample_with_limits(sample.clone(), 2, retained)
                .is_ok()
        );
        assert!(
            recorder
                .push_sample_with_limits(sample, 2, retained)
                .is_err()
        );
        assert!(validate_capture_size(MAX_CAPTURE_BYTES).is_ok());
        assert!(validate_capture_size(MAX_CAPTURE_BYTES.saturating_add(1)).is_err());
    }
}
