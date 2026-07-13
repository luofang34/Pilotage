//! Adapter traits and the capability model that engine-specific adapters
//! implement (ADR-0008).
//!
//! This crate is sans-IO: it defines the boundary traits only. Engine SDK
//! calls and I/O live in adapter implementations such as
//! `pilotage-adapter-reference`, per ADR-0002.

mod calibration;
mod capability;
mod control;
mod step;
mod telemetry;
mod vehicle_adapter;
mod video;

pub use calibration::{
    AlignmentAllowances, AlignmentErrorBudget, BodyToCameraExtrinsics, Boresight,
    BrownConradyDistortion, CALIBRATION_SCHEMA_VERSION, CalibrationError, CalibrationIdentity,
    CalibrationVersion, CameraCalibration, CameraGeometry, DesignEye, EffectivePeriod, FieldOfView,
    OpticalConvention, PinholeIntrinsics, ProvenanceSource, RecoveryReport, Residuals,
    SIM_FPV_CALIBRATION_HASH, SIM_FPV_CALIBRATION_ID, SIM_FPV_CAMERA_ID, SyntheticTarget,
    ToolVersion, ValidityStatus, Viewport, content_hash, derive_budget, radians_per_pixel,
    recover_intrinsics, sim_fpv_calibration, to_canonical, validate, verify, verify_camera,
    verify_sim_recovery,
};
pub use capability::{AdapterCapabilities, ExecutionMode, ScopeDescriptor, VehicleDescriptor};
pub use control::{ApplyOutcome, Disposition, LinkLossPolicy, RejectReason};
pub use step::{StepBudget, StepOutcome};
pub use telemetry::{
    AvionicsAttitudeSample, AvionicsKinematicsSample, AvionicsSample, MeasurementClock,
    MeasurementStamp, Pose2d, SourceIncarnation, TelemetryBatch, TelemetrySample, VideoSource,
};
pub use vehicle_adapter::VehicleAdapter;
pub use video::{CalibrationId, CameraId, CaptureClockMapping, VideoCaptureStamp};
