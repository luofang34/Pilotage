//! Immutable output from one HID characterization run.

use serde::{Deserialize, Serialize};

use crate::{AxisCalibration, DeviceInfo};

use super::capture::{DeadzoneEvidence, SamplingSource};

/// The supported calibration candidate schema.
pub const CALIBRATION_CANDIDATE_SCHEMA_VERSION: u32 = 1;

/// The measured center behavior during an idle segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CenterBehavior {
    /// The center stayed within its measured noise floor.
    Stable,
    /// The center moved beyond its measured noise floor.
    Drifting,
}

/// Report timing results for one capture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimingCharacterization {
    /// The number of reports in the timing calculation.
    pub sample_count: u64,
    /// The median report period in microseconds.
    pub median_period_us: f64,
    /// The median absolute report-period deviation in microseconds.
    pub jitter_mad_us: f64,
    /// The estimated number of reports that did not arrive.
    pub dropped_report_count: u64,
    /// Confidence in the timing result in `[0, 1]`.
    pub confidence: f32,
}

/// Characterization and proposed calibration for one named control.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AxisCharacterization {
    /// The logical control that the operator moved.
    pub logical: String,
    /// The source axis with the unique movement.
    pub source_index: usize,
    /// Whether positive physical movement produced decreasing source values.
    pub invert: bool,
    /// The proposed raw calibration.
    pub calibration: AxisCalibration,
    /// The smallest observed source value.
    pub observed_min: f32,
    /// The median source value during idle.
    pub observed_center: f32,
    /// The largest observed source value.
    pub observed_max: f32,
    /// Median absolute idle noise in source units.
    pub center_noise: f32,
    /// Absolute center movement per second in source units.
    pub center_drift_per_second: f32,
    /// The idle center classification.
    pub center_behavior: CenterBehavior,
    /// The largest other-axis excursion divided by the selected-axis excursion.
    pub cross_axis_coupling: f32,
    /// The proposed device-noise dead zone in normalized units.
    pub proposed_deadzone: f32,
    /// The number of idle reports used for this axis.
    pub idle_sample_count: u64,
    /// The number of movement reports used for this axis.
    pub movement_sample_count: u64,
    /// Confidence in this axis result in `[0, 1]`.
    pub confidence: f32,
}

/// A reviewable calibration candidate that cannot contain a feel curve or a
/// vehicle limit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationCandidate {
    /// The candidate schema version.
    pub schema_version: u32,
    /// The sampled device identity.
    pub device: DeviceInfo,
    /// The sampling port.
    pub source: SamplingSource,
    /// SHA-256 of the exact source capture bytes.
    pub source_capture_digest: String,
    /// SHA-256 of the exact baseline profile bytes.
    pub baseline_profile_digest: String,
    /// Report timing results.
    pub timing: TimingCharacterization,
    /// Evidence about platform dead-zone shaping.
    pub deadzone_evidence: DeadzoneEvidence,
    /// Proposed axis calibrations.
    pub axes: Vec<AxisCharacterization>,
    /// Total reports in the source capture.
    pub sample_count: u64,
    /// Confidence in the complete candidate in `[0, 1]`.
    pub confidence: f32,
}
