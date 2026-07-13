//! The calibration's contribution to the downstream conformal alignment error
//! budget (ADR-0021).
//!
//! A conformal overlay's total alignment error has many contributors; this type
//! publishes only the part the *calibration* is responsible for, as a single
//! conservative angular bound the next HUD increment can compose into its own
//! budget, with each component traceable. It is a SIM engineering budget, not a
//! measured optical-alignment tolerance.
//!
//! Quantities the calibration *recovered* (the intrinsic residual) are measured.
//! Quantities it *declared* rather than recovered (the extrinsics, boresight,
//! design eye, and the pinhole/no-distortion model assumption) each carry an
//! explicit engineering allowance with a stated rationale — never zero, because
//! "declared exactly" in the sim world is still an assumption a real integration
//! would have to bound. The total is a conservative linear (worst-case, not
//! root-sum-square) sum, so a consumer that adds it to its own budget never
//! under-counts.

use super::geometry::CameraGeometry;

/// Distortion / model-mismatch allowance, in pixels. The sim camera is an ideal
/// pinhole with no distortion; a real capture path or any residual model
/// mismatch is bounded to sub-pixel, so a half-pixel allowance is charged rather
/// than claiming zero model error.
pub const DISTORTION_MODEL_ALLOWANCE_PX: f64 = 0.5;

/// Extrinsics rotation allowance, in radians. The body-to-camera rotation is
/// declared from the sim mount pose, not recovered; a declared mounting is
/// allowed a few milliradians of orientation error as an engineering allowance.
pub const EXTRINSICS_ROTATION_ALLOWANCE_RAD: f64 = 0.005;

/// Boresight allowance, in radians. The boresight is declared as the optical
/// axis, not measured; a couple of milliradians are charged for that.
pub const BORESIGHT_ALLOWANCE_RAD: f64 = 0.002;

/// Design-eye position allowance, in meters. The design eye is declared
/// coincident with the optical center; a centimeter of positional uncertainty
/// is charged rather than claiming an exact eye point.
pub const DESIGN_EYE_ALLOWANCE_M: f64 = 0.01;

/// Reference range, in meters, at which the design-eye positional allowance is
/// converted to an angular parallax bound. A representative near-field conformal
/// feature range for the SIM rover.
pub const DESIGN_EYE_REFERENCE_RANGE_M: f64 = 50.0;

/// The calibration's contribution to the conformal alignment error budget. All
/// bounds are conservative; the total is a worst-case linear sum.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlignmentErrorBudget {
    /// Recovered intrinsic reprojection residual (max), in pixels. Measured.
    pub intrinsic_residual_px: f64,
    /// Distortion / model-mismatch allowance, in pixels. Declared.
    pub distortion_model_allowance_px: f64,
    /// Extrinsics rotation allowance, in radians. Declared.
    pub extrinsics_rotation_allowance_rad: f64,
    /// Boresight allowance, in radians. Declared.
    pub boresight_allowance_rad: f64,
    /// Design-eye parallax allowance, in radians (positional allowance at the
    /// reference range). Declared.
    pub design_eye_allowance_rad: f64,
    /// Pixel-to-angle conversion actually used, in radians per pixel.
    pub radians_per_pixel: f64,
    /// Conservative total pixel bound: intrinsic residual plus the distortion
    /// allowance, in pixels.
    pub total_pixel_bound_px: f64,
    /// Conservative total angular bound, in radians: the pixel bound converted
    /// to angle plus the declared angular allowances, summed worst-case. This is
    /// the single number a downstream budget composes.
    pub total_angular_bound_rad: f64,
}

/// Derives the alignment error budget from a camera's geometry and its measured
/// intrinsic residual. The pixel-to-angle factor uses the smaller focal length
/// (the larger angle per pixel), for conservatism.
#[must_use]
pub fn derive_budget(
    geometry: &CameraGeometry,
    intrinsic_residual_px: f64,
) -> AlignmentErrorBudget {
    let focal = geometry
        .intrinsics
        .focal_x_px
        .min(geometry.intrinsics.focal_y_px);
    let radians_per_pixel = 1.0 / focal;
    let design_eye_allowance_rad = DESIGN_EYE_ALLOWANCE_M / DESIGN_EYE_REFERENCE_RANGE_M;
    let total_pixel_bound_px = intrinsic_residual_px + DISTORTION_MODEL_ALLOWANCE_PX;
    let total_angular_bound_rad = total_pixel_bound_px * radians_per_pixel
        + EXTRINSICS_ROTATION_ALLOWANCE_RAD
        + BORESIGHT_ALLOWANCE_RAD
        + design_eye_allowance_rad;
    AlignmentErrorBudget {
        intrinsic_residual_px,
        distortion_model_allowance_px: DISTORTION_MODEL_ALLOWANCE_PX,
        extrinsics_rotation_allowance_rad: EXTRINSICS_ROTATION_ALLOWANCE_RAD,
        boresight_allowance_rad: BORESIGHT_ALLOWANCE_RAD,
        design_eye_allowance_rad,
        radians_per_pixel,
        total_pixel_bound_px,
        total_angular_bound_rad,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{BORESIGHT_ALLOWANCE_RAD, EXTRINSICS_ROTATION_ALLOWANCE_RAD, derive_budget};
    use crate::calibration::sim_fpv_calibration;

    #[test]
    fn budget_totals_are_conservative_linear_sums() {
        let cal = sim_fpv_calibration();
        let b = derive_budget(&cal.geometry, cal.identity.residuals.max_px);
        assert!(
            (b.total_pixel_bound_px - (b.intrinsic_residual_px + b.distortion_model_allowance_px))
                .abs()
                < 1e-12
        );
        let expected_angular = b.total_pixel_bound_px * b.radians_per_pixel
            + b.extrinsics_rotation_allowance_rad
            + b.boresight_allowance_rad
            + b.design_eye_allowance_rad;
        assert!((b.total_angular_bound_rad - expected_angular).abs() < 1e-12);
    }

    #[test]
    fn declared_allowances_are_never_zero() {
        let cal = sim_fpv_calibration();
        let b = derive_budget(&cal.geometry, cal.identity.residuals.max_px);
        assert!(b.distortion_model_allowance_px > 0.0);
        assert_eq!(
            b.extrinsics_rotation_allowance_rad,
            EXTRINSICS_ROTATION_ALLOWANCE_RAD
        );
        assert_eq!(b.boresight_allowance_rad, BORESIGHT_ALLOWANCE_RAD);
        assert!(b.design_eye_allowance_rad > 0.0);
        assert!(b.total_angular_bound_rad > 0.0);
    }
}
