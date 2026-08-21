//! Portable samples for automatic HID characterization.

use serde::{Deserialize, Serialize};

use crate::DeviceInfo;

/// The supported characterization capture schema.
pub const CHARACTERIZATION_CAPTURE_SCHEMA_VERSION: u32 = 1;

/// The port that produced the samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SamplingSource {
    /// A raw HID report received through the Apple native port.
    AppleHid,
    /// A raw HID report received through a non-Apple native port.
    NativeHid,
    /// A browser Gamepad API sample.
    BrowserGamepad,
    /// A deterministic test source.
    Synthetic,
}

/// The timestamp used for report timing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimestampSource {
    /// The sampling port supplied a source timestamp.
    Source,
    /// The bridge recorded only an arrival timestamp.
    Arrival,
}

/// Whether a platform layer has already applied a physical dead zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeadzoneEvidenceStatus {
    /// The capture did not measure this property.
    Unknown,
    /// The samples show a platform dead zone.
    Observed,
    /// The samples show raw motion without a platform dead zone.
    NotObserved,
}

/// The method that produced dead-zone evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeadzoneEvidenceMethod {
    /// The port did not compare physical and platform values.
    Unmeasured,
    /// The port recorded raw HID reports before platform shaping.
    RawHidReports,
    /// The port compared native HID and platform values for the same motion.
    PairedNativeAndPlatform,
}

/// Evidence about a platform dead zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeadzoneEvidence {
    /// The measured result.
    pub status: DeadzoneEvidenceStatus,
    /// The measurement method.
    pub method: DeadzoneEvidenceMethod,
    /// The number of paired or raw reports in the measurement.
    pub sample_count: u64,
}

/// One decoded device sample.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureSample {
    /// A capture-local monotonic sequence.
    pub sequence: u64,
    /// Microseconds from capture start at bridge receipt.
    pub observed_at_us: u64,
    /// Microseconds from the source clock, when the port supplies it.
    pub source_at_us: Option<u64>,
    /// Axis values in the source port's units.
    pub axes: Vec<f32>,
    /// The raw report bytes, when the port supplies them.
    pub report_hex: Option<String>,
}

/// The operator action for one capture segment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaptureSegmentKind {
    /// The operator does not move a control.
    Idle,
    /// The operator moves one named control in the positive direction first.
    Movement {
        /// The logical control name.
        logical: String,
        /// True when the operator moved the positive direction first.
        positive_first: bool,
    },
}

/// A closed sequence range with one operator action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureSegment {
    /// The operator action.
    pub action: CaptureSegmentKind,
    /// The first included sample sequence.
    pub start_sequence: u64,
    /// The last included sample sequence.
    pub end_sequence: u64,
}

/// A portable characterization capture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterizationCapture {
    /// The capture schema version.
    pub schema_version: u32,
    /// The sampled device identity.
    pub device: DeviceInfo,
    /// The sampling port.
    pub source: SamplingSource,
    /// The clock selected for timing analysis.
    pub timestamp_source: TimestampSource,
    /// Evidence about platform dead-zone shaping.
    pub deadzone_evidence: DeadzoneEvidence,
    /// Samples in sequence order.
    pub samples: Vec<CaptureSample>,
    /// Operator actions over closed sequence ranges.
    pub segments: Vec<CaptureSegment>,
}
