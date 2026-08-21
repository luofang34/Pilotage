//! HID characterization evidence and calibration candidate contracts.

mod analysis;
mod candidate;
mod capture;
mod promotion;
mod raw_report;

pub use analysis::{AnalysisError, characterize_capture};
pub use candidate::{
    AxisCharacterization, CALIBRATION_CANDIDATE_SCHEMA_VERSION, CalibrationCandidate,
    CenterBehavior, TimingCharacterization,
};
pub use capture::{
    CHARACTERIZATION_CAPTURE_SCHEMA_VERSION, CaptureSample, CaptureSegment, CaptureSegmentKind,
    CharacterizationCapture, DeadzoneEvidence, DeadzoneEvidenceMethod, DeadzoneEvidenceStatus,
    MAX_CHARACTERIZATION_CAPTURE_BYTES, MAX_CHARACTERIZATION_CAPTURE_SAMPLES,
    MAX_CHARACTERIZATION_PRODUCT_NAME_BYTES, MAX_CHARACTERIZATION_RAW_REPORT_BYTES,
    NeutralPosition, RawReportAxisField, RawReportLayout, SOURCE_AXIS_CONTRACT_SCHEMA_VERSION,
    SamplingSource, SourceAxisContract, SourceAxisRange, TimestampSource, TimingObservation,
};
pub use promotion::{
    CharacterizationError, PromotionConfirmation, canonical_candidate_digest,
    promote_calibration_candidate,
};
pub use raw_report::{RawReportDecoder, RawReportError};

#[cfg(test)]
#[path = "characterization/tests.rs"]
mod tests;
