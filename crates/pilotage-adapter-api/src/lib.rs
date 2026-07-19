//! Adapter traits and the capability model that engine-specific adapters
//! implement (ADR-0008).
//!
//! This crate is sans-IO: it defines the boundary traits only. Engine SDK
//! calls and I/O live in adapter implementations such as
//! `pilotage-adapter-reference`, per ADR-0002.

mod capability;
mod control;
mod step;
mod telemetry;
mod vehicle_adapter;
mod video;

pub use capability::{AdapterCapabilities, ExecutionMode, ScopeDescriptor, VehicleDescriptor};
pub use control::{
    ApplyOutcome, Disposition, LinkLossEnactError, LinkLossPolicy, RejectReason,
    payload_satisfies_neutral_activation,
};
// CAL-01 (#90): the camera calibration contract moved to
// `pilotage-camera-calibration`; re-export it so `pilotage_adapter_api::…` paths
// for consumers are unchanged, and expose the new `VerifiedCameraModel`.
pub use pilotage_camera_calibration::{
    AlignmentAllowances, AlignmentErrorBudget, BodyToCameraExtrinsics, Boresight,
    BrownConradyDistortion, CALIBRATION_SCHEMA_VERSION, CalibrationError, CalibrationIdentity,
    CalibrationVersion, CameraCalibration, CameraGeometry, DesignEye, EffectivePeriod, FieldOfView,
    OpticalConvention, PinholeIntrinsics, ProvenanceSource, RecoveryReport, Residuals,
    SIM_FPV_CALIBRATION_HASH, SIM_FPV_CALIBRATION_ID, SIM_FPV_CAMERA_ID, SyntheticTarget,
    ToolVersion, ValidityStatus, VerifiedCameraModel, Viewport, content_hash, derive_budget,
    radians_per_pixel, recover_intrinsics, sim_fpv_calibration, to_canonical, validate, verify,
    verify_camera, verify_sim_recovery,
};
pub use step::{StepBudget, StepOutcome};
pub use telemetry::{
    AvionicsAttitudeSample, AvionicsKinematicsSample, AvionicsSample, FcStateSample,
    MeasurementClock, MeasurementStamp, Pose2d, SimTruthSample, SourceIncarnation, SourceIntegrity,
    SourceRole, TelemetryBatch, TelemetrySample, VideoSource,
};
pub use vehicle_adapter::VehicleAdapter;
pub use video::{CalibrationId, CameraId, CaptureClockMapping, VideoCaptureStamp};
