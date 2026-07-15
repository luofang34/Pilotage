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
//! effective window, provenance, residuals, validity status) and its
//! [`budget`] contribution to the conformal alignment error. Each artifact is
//! hashed over a fixed [`canonical`] byte form; [`verify`] recomputes the hash
//! **and** runs [`validate`] over every geometry, lifecycle, and budget
//! invariant, so a hash-consistent but semantically invalid artifact still
//! fails closed. The single published simulator artifact lives in [`sim`], and
//! [`recovery`] is the deterministic tool that demonstrates the fit recovers
//! known parameters from synthetic targets within a documented tolerance.

mod budget;
mod canonical;
mod error;
mod geometry;
mod identity;
mod recovery;
mod sim;
mod validate;
mod verified;

use pilotage_frames::Quat;

pub use budget::{AlignmentAllowances, AlignmentErrorBudget, derive_budget, radians_per_pixel};
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
pub use validate::validate;
pub use verified::VerifiedCameraModel;

/// A complete calibration artifact: a simulated camera's geometry, its identity
/// and lifecycle metadata, and the stored alignment-error allowances.
///
/// Derivable quantities are not stored: the field of view follows from
/// [`CameraGeometry::field_of_view`], and the alignment budget totals follow
/// from [`Self::budget`]. Only the irreducible inputs live in the artifact.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraCalibration {
    /// Camera geometry (intrinsics, distortion, viewport, extrinsics, design
    /// eye, boresight). The field of view is derived, not stored.
    pub geometry: CameraGeometry,
    /// Identity, lifecycle, provenance, and residuals.
    pub identity: CalibrationIdentity,
    /// The stored alignment-error allowances; the full budget is derived by
    /// [`Self::budget`].
    pub allowances: AlignmentAllowances,
}

impl CameraCalibration {
    /// The SHA-256 content hash of this calibration's canonical form.
    #[must_use]
    pub fn content_hash(&self) -> [u8; 32] {
        content_hash(self)
    }

    /// The alignment error budget, derived from the stored allowances and the
    /// camera's focal lengths.
    #[must_use]
    pub fn budget(&self) -> AlignmentErrorBudget {
        derive_budget(&self.geometry.intrinsics, &self.allowances)
    }

    /// Verifies the artifact for use at `now_unix_ns` against its
    /// `recorded_hash`: hash match, semantic validity, status, and effective
    /// window.
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

    /// Validates every geometry, lifecycle, and budget invariant, independent of
    /// the hash.
    ///
    /// # Errors
    ///
    /// The first failing [`CalibrationError`].
    pub fn validate(&self) -> Result<(), CalibrationError> {
        validate(self)
    }

    /// Verifies this calibration applies to a frame from `frame_camera_id`.
    ///
    /// # Errors
    ///
    /// [`CalibrationError::WrongCamera`] on a mismatch.
    pub fn verify_camera(&self, frame_camera_id: u32) -> Result<(), CalibrationError> {
        verify_camera(self, frame_camera_id)
    }

    /// Verifies the artifact against `recorded_hash` at `now_unix_ns` and, on
    /// success, mints the [`VerifiedCameraModel`] a conformal projector derives
    /// its view geometry from.
    ///
    /// This is the **sole** path to a [`VerifiedCameraModel`]: its constructor is
    /// crate-private and reached only here, after [`Self::verify`] recomputes the
    /// content hash and runs every semantic check. So a `VerifiedCameraModel`
    /// cannot exist without a genuine hash-verification of the artifact it
    /// describes. The resolved geometry is read from the same verified artifact:
    /// the body→camera extrinsics rotation, the field-of-view half-tangents
    /// `((w/2)/fx, (h/2)/fy)`, and the published alignment bound.
    ///
    /// # Errors
    ///
    /// The first failing [`CalibrationError`] from [`Self::verify`].
    pub fn verified_camera_model(
        &self,
        recorded_hash: [u8; 32],
        now_unix_ns: u64,
    ) -> Result<VerifiedCameraModel, CalibrationError> {
        self.verify(recorded_hash, now_unix_ns)?;
        let intrinsics = &self.geometry.intrinsics;
        let viewport = &self.geometry.viewport;
        let [w, x, y, z] = self.geometry.extrinsics.rotation_quat_wxyz;
        Ok(VerifiedCameraModel::new(
            self.identity.calibration_id,
            self.content_hash(),
            Quat {
                w: w as f32,
                x: x as f32,
                y: y as f32,
                z: z as f32,
            },
            (
                (f64::from(viewport.width_px) / 2.0) / intrinsics.focal_x_px,
                (f64::from(viewport.height_px) / 2.0) / intrinsics.focal_y_px,
            ),
            self.budget().total_angular_bound_rad,
        ))
    }
}

#[cfg(test)]
mod tests;
