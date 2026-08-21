//! Portable samples for automatic HID characterization.

use serde::{Deserialize, Deserializer, Serialize};

use crate::DeviceInfo;

/// The supported characterization capture schema.
pub const CHARACTERIZATION_CAPTURE_SCHEMA_VERSION: u32 = 1;
/// The supported source-axis contract schema.
pub const SOURCE_AXIS_CONTRACT_SCHEMA_VERSION: u32 = 1;
/// The maximum report count in one characterization capture.
pub const MAX_CHARACTERIZATION_CAPTURE_SAMPLES: usize = 1_000_000;
/// The maximum encoded size of one characterization capture.
pub const MAX_CHARACTERIZATION_CAPTURE_BYTES: usize = 64 * 1024 * 1024;
/// The maximum UTF-8 byte count of a product name in characterization evidence.
pub const MAX_CHARACTERIZATION_PRODUCT_NAME_BYTES: usize = 256;
/// The maximum raw HID report size accepted by a source-axis contract.
pub const MAX_CHARACTERIZATION_RAW_REPORT_BYTES: usize = 2_048;

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

/// The event that one timing sample represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimingObservation {
    /// Each sample came from one raw report callback.
    ReportCallbacks,
    /// Each sample came from one changed state seen by a polling API.
    PolledStateUpdates,
    /// Each sample was supplied by a test or an untrusted draft producer.
    InjectedSamples,
}

/// The trusted source-unit range for one decoded axis.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceAxisRange {
    /// The axis index in each capture sample.
    pub source_index: usize,
    /// The smallest value declared by the source contract.
    pub minimum: f32,
    /// The largest value declared by the source contract.
    pub maximum: f32,
    /// The physical control position when the operator releases it.
    pub neutral_position: NeutralPosition,
}

/// Trusted axis ranges and neutral positions for one device port.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceAxisContract {
    /// The source-axis contract schema version.
    pub schema_version: u32,
    /// The device identity that owns the axis contract.
    pub device: DeviceInfo,
    /// The raw report decoder for a native HID source.
    #[serde(default)]
    pub raw_report_layout: Option<RawReportLayout>,
    /// The source-unit contract for each decoded axis.
    pub axes: Vec<SourceAxisRange>,
}

/// A bounded raw HID report layout.
///
/// Bit zero is the least-significant bit of report byte zero. Bit offsets
/// increase through each byte and then through the report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawReportLayout {
    /// The exact byte count of one report.
    pub report_byte_count: usize,
    /// The required report ID in byte zero, when the device uses report IDs.
    #[serde(default)]
    pub report_id: Option<u8>,
    /// One raw bit field for each decoded source axis.
    pub axes: Vec<RawReportAxisField>,
}

/// One integer axis field in a raw HID report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawReportAxisField {
    /// The output axis index.
    pub source_index: usize,
    /// The first field bit in the report's least-significant-bit-first stream.
    pub bit_offset: usize,
    /// The integer field width. The supported range is 1 through 24 bits.
    pub bit_width: u8,
    /// True when the field uses two's-complement signed form.
    pub signed: bool,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum CaptureSegmentKindWire {
    Idle(IdleAction),
    Movement(MovementAction),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdleAction {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MovementAction {
    logical: String,
    positive_first: bool,
}

impl<'de> Deserialize<'de> for CaptureSegmentKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match CaptureSegmentKindWire::deserialize(deserializer)? {
            CaptureSegmentKindWire::Idle(_) => Ok(Self::Idle),
            CaptureSegmentKindWire::Movement(action) => Ok(Self::Movement {
                logical: action.logical,
                positive_first: action.positive_first,
            }),
        }
    }
}

/// The physical control position when the operator releases the control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NeutralPosition {
    /// The control returns between its two endpoints.
    Centered,
    /// The control stays at its minimum endpoint.
    Minimum,
    /// The control stays at its maximum endpoint.
    Maximum,
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
    /// A token for one connection or one open device handle.
    pub device_instance_id: String,
    /// The sampling port.
    pub source: SamplingSource,
    /// The clock selected for timing analysis.
    pub timestamp_source: TimestampSource,
    /// The event represented by each timing sample.
    pub timing_observation: TimingObservation,
    /// Evidence about platform dead-zone shaping.
    pub deadzone_evidence: DeadzoneEvidence,
    /// SHA-256 of the exact trusted source-axis contract bytes.
    pub source_contract_digest: String,
    /// Trusted source-unit ranges for all decoded axes.
    pub source_axes: Vec<SourceAxisRange>,
    /// Samples in sequence order.
    pub samples: Vec<CaptureSample>,
    /// Operator actions over closed sequence ranges.
    pub segments: Vec<CaptureSegment>,
}
