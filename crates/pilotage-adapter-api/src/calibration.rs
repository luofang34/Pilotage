//! Versioned, hashed, fail-closed calibration for a SIMULATED camera and its
//! design-eye reference (ADR-0021).
//!
//! # This is SIM, not optical HUD qualification
//!
//! Everything here describes a *simulated* pinhole camera and a *synthetic*
//! design-eye reference so a conformal overlay can be projected consistently
//! in the simulator. It is deliberately **not** a model of a real head-up
//! display: there is no combiner, no collimation, no eyebox, no installation
//! alignment, and no optical qualification. No artifact, field, or document
//! here may be read as HUD airworthiness or optical qualification. SIM / NOT
//! FOR FLIGHT.
//!
//! # Contract
//!
//! A [`CameraCalibration`] pairs the camera [`geometry`] (every unit and frame
//! explicit) with the [`identity`] and lifecycle (id, version, tool version,
//! effective window, provenance, residuals, validity status). Each artifact is
//! hashed over a fixed [`canonical`] byte form; [`verify`] recomputes and
//! compares, so a mutated value fails closed. The single published simulator
//! artifact lives in [`sim`], and [`recovery`] is the deterministic tool that
//! demonstrates the fit recovers known parameters from synthetic targets
//! within a documented tolerance.

mod canonical;
mod error;
mod geometry;
mod identity;
mod recovery;
mod sim;

pub use canonical::{
    CALIBRATION_SCHEMA_VERSION, content_hash, to_canonical, verify, verify_camera,
};
pub use error::CalibrationError;
pub use geometry::{
    BodyToCameraExtrinsics, Boresight, BrownConradyDistortion, CameraGeometry, DesignEye,
    FieldOfView, OpticalConvention, PinholeIntrinsics, Viewport,
};
pub use identity::{
    CalibrationIdentity, CalibrationVersion, EffectivePeriod, ProvenanceSource, Residuals,
    ToolVersion, ValidityStatus,
};
pub use recovery::{RecoveryReport, SyntheticTarget, recover_intrinsics, verify_sim_recovery};
pub use sim::{
    SIM_FPV_CALIBRATION_HASH, SIM_FPV_CALIBRATION_ID, SIM_FPV_CAMERA_ID, sim_fpv_calibration,
};

/// A complete calibration artifact: a simulated camera's geometry plus its
/// identity and lifecycle metadata.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraCalibration {
    /// Camera geometry (intrinsics, distortion, viewport, FOV, extrinsics,
    /// design eye, boresight).
    pub geometry: CameraGeometry,
    /// Identity, lifecycle, provenance, and residuals.
    pub identity: CalibrationIdentity,
}

impl CameraCalibration {
    /// The SHA-256 content hash of this calibration's canonical form.
    #[must_use]
    pub fn content_hash(&self) -> [u8; 32] {
        content_hash(self)
    }

    /// Verifies the artifact for use at `now_unix_ns` against its
    /// `recorded_hash` (hash match, status, effective window).
    ///
    /// # Errors
    ///
    /// The first failing [`CalibrationError`].
    pub fn verify(
        &self,
        recorded_hash: [u8; 32],
        now_unix_ns: u64,
    ) -> Result<(), CalibrationError> {
        verify(self, recorded_hash, now_unix_ns)
    }

    /// Verifies this calibration applies to a frame from `frame_camera_id`.
    ///
    /// # Errors
    ///
    /// [`CalibrationError::WrongCamera`] on a mismatch.
    pub fn verify_camera(&self, frame_camera_id: u32) -> Result<(), CalibrationError> {
        verify_camera(self, frame_camera_id)
    }
}

#[cfg(test)]
mod tests;
