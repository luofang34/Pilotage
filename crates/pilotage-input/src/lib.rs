//! Canonical input model, device profiles, and the normalization pipeline
//! that maps physical device state onto `pilotage-protocol` control frames
//! (ADR-0007).
//!
//! This crate is sans-IO: device polling and OS input APIs live in platform
//! ports, per ADR-0002.
//!
//! Pipeline stage order (ADR-0007): a [`RawDeviceSample`] is normalized per
//! axis by [`normalize_axis`], its buttons are edge-detected by
//! [`ButtonTracker`], and both are bound to logical IDs via
//! [`axis_id_for_name`]/[`button_id_for_name`] using the effective
//! [`DeviceProfile`] produced by [`merge_layers`].

mod button_tracker;
mod characterization;
mod digest;
mod logical;
mod normalize;
mod profile;
mod registry;
mod sample;

pub use button_tracker::ButtonTracker;
pub use characterization::{
    AnalysisError, AxisCharacterization, CALIBRATION_CANDIDATE_SCHEMA_VERSION,
    CHARACTERIZATION_CAPTURE_SCHEMA_VERSION, CalibrationCandidate, CaptureSample, CaptureSegment,
    CaptureSegmentKind, CenterBehavior, CharacterizationCapture, CharacterizationError,
    DeadzoneEvidence, DeadzoneEvidenceMethod, DeadzoneEvidenceStatus,
    MAX_CHARACTERIZATION_CAPTURE_BYTES, MAX_CHARACTERIZATION_CAPTURE_SAMPLES,
    MAX_CHARACTERIZATION_PRODUCT_NAME_BYTES, MAX_CHARACTERIZATION_RAW_REPORT_BYTES,
    NeutralPosition, PromotionConfirmation, RawReportAxisField, RawReportDecoder, RawReportError,
    RawReportLayout, SOURCE_AXIS_CONTRACT_SCHEMA_VERSION, SamplingSource, SourceAxisContract,
    SourceAxisRange, TimestampSource, TimingCharacterization, TimingObservation,
    canonical_candidate_digest, characterize_capture, promote_calibration_candidate,
};
pub use digest::{DIGEST_LEN, content_digest};
pub use logical::{SLOT_AXIS_BASE, SLOT_AXIS_COUNT};
pub use logical::{axis_id_for_name, button_id_for_name};
pub use normalize::{NormalizedAxis, normalize_axis};
pub use profile::{
    AxisCalibration, AxisConfig, ButtonConfig, DeviceIdentity, DeviceInfo, DeviceProfile,
    KeyAxisBinding, KeyBinding, ProfileError, SCHEMA_VERSION, parse_profile_bytes,
    parse_profile_str, validate_axis_config, validate_physical_axis_config,
};
pub use registry::{
    GENERIC_GAMEPAD_JSON, LayeredProfile, ProfileLayer, SelectError, layered,
    load_builtin_generic_gamepad, load_profile_bytes, load_profile_str, merge_layers,
    select_by_identity,
};
pub use sample::RawDeviceSample;
