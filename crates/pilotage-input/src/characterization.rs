//! HID characterization evidence and calibration candidate contracts.

mod candidate;
mod capture;
mod promotion;

pub use candidate::{
    AxisCharacterization, CALIBRATION_CANDIDATE_SCHEMA_VERSION, CalibrationCandidate,
    CenterBehavior, TimingCharacterization,
};
pub use capture::{
    CHARACTERIZATION_CAPTURE_SCHEMA_VERSION, CaptureSample, CaptureSegment, CaptureSegmentKind,
    CharacterizationCapture, DeadzoneEvidence, DeadzoneEvidenceMethod, DeadzoneEvidenceStatus,
    SamplingSource, TimestampSource,
};
pub use promotion::{
    CharacterizationError, PromotionConfirmation, canonical_candidate_digest,
    promote_calibration_candidate,
};
