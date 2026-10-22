//! Adapter traits and the capability model that engine-specific adapters
//! implement (ADR-0008).
//!
//! This crate is sans-IO: it defines the boundary traits only. Engine SDK
//! calls and I/O live in adapter implementations such as
//! `pilotage-adapter-reference`, per ADR-0002.

mod authority;
mod capability;
mod control;
mod flight_procedure;
mod payload;
mod step;
mod telemetry;
mod vehicle_adapter;
mod video;

pub use authority::ControlAuthorityLoss;
pub use capability::{
    ActionCapability, AdapterCapabilities, ControlFeelDescriptor, ControlFeelMode, ExecutionMode,
    IntentCapability, LegacyAxisRoute, LegacyCommandMap, SIM_LIFECYCLE_SCOPE, ScopeDescriptor,
    VehicleDescriptor, sim_lifecycle_descriptor,
};
pub use control::{
    ActionResult, ApplyOutcome, Disposition, LinkLossEnactError, LinkLossPolicy, RejectReason,
    attitude_euler, intent_satisfies_neutral_activation, link_loss_latch_engaged,
    payload_satisfies_neutral_activation,
};
pub use flight_procedure::{
    FlightProcedureKind, FlightProcedurePhase, FlightProcedureReason, FlightProcedureRequest,
    FlightProcedureSample,
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
// The typed vocabulary an adapter reports, shared with clients.
pub use pilotage_protocol::{
    CommandOutcome, GimbalAxisLocks, GimbalReference, GimbalYawFrame, HeadingReference,
    LandedState, MagneticVariationSource, MotorState, VehicleClass,
};
pub use step::{StepBudget, StepOutcome};
pub use telemetry::{
    AvionicsAttitudeSample, AvionicsBaroSample, AvionicsKinematicsSample, AvionicsSample,
    BatterySample, FcCommandAck, FcStateSample, GeodeticFixSample, GimbalAttitudeSample,
    LandedStateSample, LandingApproachSample, MagneticVariationSample, MeasurementClock,
    MeasurementStamp, MotorStateSample, Pose2d, SimTruthSample, SourceIncarnation, SourceIntegrity,
    SourceRole, TelemetryBatch, TelemetrySample, VideoSource,
};
pub use vehicle_adapter::VehicleAdapter;
pub use video::stamping::{
    CHASE_CAMERA, CHASE_SOURCE_ID, FPV_CAMERA, FPV_SOURCE_ID, FrameStamper, GIMBAL_CAMERA,
    GIMBAL_SOURCE_ID, RawVideoFrame, UnstampedFrame,
};
pub use video::{CalibrationId, CameraId, CaptureClockMapping, VideoCaptureStamp};

pub use payload::{
    payload_angle_observation_to_wire, payload_capabilities_to_wire, payload_progress_to_wire,
    payload_rate_observation_to_wire, payload_zoom_observation_to_wire,
};
