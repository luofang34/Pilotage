//! Versioned, hashed, fail-closed calibration for a SIMULATED camera and its
//! design-eye reference (ADR-0021, CAL-01), and the verified camera model a
//! conformal HUD derives its view geometry from.
//!
//! # This is SIM, not optical HUD qualification
//!
//! Everything here describes a *simulated* pinhole camera and a *synthetic*
//! design-eye reference so a conformal overlay can be projected consistently in
//! the simulator. It is deliberately **not** a model of a real head-up display,
//! and nothing here may be read as HUD airworthiness or optical qualification.
//! SIM / NOT FOR FLIGHT.
//!
//! # Unforgeable verified model
//!
//! A [`VerifiedCameraModel`] is the resolved geometry of a calibration artifact.
//! It has no public constructor: the only way to obtain one is
//! [`CameraCalibration::verified_camera_model`], which recomputes the content
//! hash and runs every semantic check before minting through a crate-private
//! constructor. Possessing a `VerifiedCameraModel` is therefore proof that a
//! genuine hash-verification succeeded — a downstream conformal projector can
//! consume it without re-hashing and cannot be handed a fabricated one. This
//! crate is `no_std` (+ `alloc`) so a foundational, `no_std` projector can depend
//! on the type while the hashing lives here.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

#[cfg(test)]
extern crate std;

mod calibration;

pub use calibration::{
    AlignmentAllowances, AlignmentErrorBudget, BodyToCameraExtrinsics, Boresight,
    BrownConradyDistortion, CALIBRATION_SCHEMA_VERSION, CalibrationError, CalibrationIdentity,
    CalibrationVersion, CameraCalibration, CameraGeometry, DesignEye, EffectivePeriod, FieldOfView,
    OpticalConvention, PinholeIntrinsics, ProvenanceSource, RecoveryReport, Residuals,
    SIM_FPV_CALIBRATION_HASH, SIM_FPV_CALIBRATION_ID, SIM_FPV_CAMERA_ID, SyntheticTarget,
    ToolVersion, ValidityStatus, VerifiedCameraModel, Viewport, content_hash, derive_budget,
    radians_per_pixel, recover_intrinsics, sim_fpv_calibration, to_canonical, validate, verify,
    verify_camera, verify_sim_recovery,
};
