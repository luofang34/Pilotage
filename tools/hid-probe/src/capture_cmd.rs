//! Guided native HID capture for automatic characterization.

use std::path::Path;
use std::time::{Duration, Instant};

use hidapi::{HidApi, HidDevice};
use pilotage_input::{
    CHARACTERIZATION_CAPTURE_SCHEMA_VERSION, CaptureSample, CaptureSegment, CaptureSegmentKind,
    CharacterizationCapture, DeadzoneEvidence, DeadzoneEvidenceMethod, DeadzoneEvidenceStatus,
    DeviceInfo, SamplingSource, TimestampSource,
};

use crate::artifact_file;
use crate::decode::to_hex;
use crate::device::{REPORT_LEN, TARGET_PRODUCT_ID, TARGET_VENDOR_ID, decode_axes};
use crate::error::ProbeError;
use crate::output::print_line;
use crate::read_cmd::REPORT_BUF_LEN;

const READ_TIMEOUT_MS: i32 = 200;

struct Recorder {
    start: Instant,
    next_sequence: u64,
    samples: Vec<CaptureSample>,
    segments: Vec<CaptureSegment>,
}

impl Recorder {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            next_sequence: 0,
            samples: Vec::new(),
            segments: Vec::new(),
        }
    }

    fn record_segment(
        &mut self,
        device: &HidDevice,
        seconds: u64,
        action: CaptureSegmentKind,
    ) -> Result<(), ProbeError> {
        let first = self.next_sequence;
        let deadline = Duration::from_secs(seconds);
        let phase_start = Instant::now();
        let mut report = [0u8; REPORT_BUF_LEN];
        while phase_start.elapsed() < deadline {
            let read = device.read_timeout(&mut report, READ_TIMEOUT_MS)?;
            if read == 0 {
                continue;
            }
            let axes =
                decode_axes(&report[..read]).map_err(|actual| ProbeError::InvalidReportLength {
                    actual,
                    expected: REPORT_LEN,
                })?;
            self.samples.push(CaptureSample {
                sequence: self.next_sequence,
                observed_at_us: elapsed_micros(self.start),
                source_at_us: None,
                axes,
                report_hex: Some(to_hex(&report[..read])),
            });
            self.next_sequence = self.next_sequence.wrapping_add(1);
        }
        let end_sequence =
            self.next_sequence
                .checked_sub(1)
                .ok_or_else(|| ProbeError::InvalidCapture {
                    detail: "a guided segment received no HID report".to_owned(),
                })?;
        self.segments.push(CaptureSegment {
            action,
            start_sequence: first,
            end_sequence,
        });
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
    let device = api.open(TARGET_VENDOR_ID, TARGET_PRODUCT_ID)?;
    let product = device.get_product_string()?;
    let mut recorder = Recorder::new();
    print_line("Keep all controls at rest.");
    recorder.record_segment(&device, idle_seconds, CaptureSegmentKind::Idle)?;
    for logical in logical_axes {
        print_line(&format!(
            "Move only {logical}. Move positive first. Use the full range. Return to center."
        ));
        recorder.record_segment(
            &device,
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
        source: native_source(),
        timestamp_source: TimestampSource::Arrival,
        deadzone_evidence: DeadzoneEvidence {
            status: DeadzoneEvidenceStatus::NotObserved,
            method: DeadzoneEvidenceMethod::RawHidReports,
            sample_count,
        },
        samples: recorder.samples,
        segments: recorder.segments,
    };
    write_capture(&capture, out)
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

fn write_capture(capture: &CharacterizationCapture, out: &Path) -> Result<(), ProbeError> {
    let json = serde_json::to_string_pretty(capture)
        .map_err(|source| ProbeError::CaptureSerialize { source })?;
    artifact_file::write_new(out, format!("{json}\n").as_bytes())
}
