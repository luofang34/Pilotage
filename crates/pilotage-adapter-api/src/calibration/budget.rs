//! The calibration's contribution to the downstream conformal alignment error
//! budget (ADR-0021).
//!
//! A conformal overlay's total alignment error has many contributors; this
//! publishes only the part the *calibration* is responsible for. Two layers:
//!
//! - [`AlignmentAllowances`] is what the artifact **stores** — the declared and
//!   measured error components. These are irreducible inputs, not derivable from
//!   anything else.
//! - [`AlignmentErrorBudget`] is **derived** from the allowances and the
//!   camera's focal lengths ([`derive_budget`]); the pixel→angle factor and the
//!   totals are computed, never stored. A total that cannot be stored cannot be
//!   made to understate the components it should sum.
//!
//! Quantities the calibration *recovered* (the intrinsic residual) are measured.
//! Quantities it *declared* rather than recovered (the extrinsics, boresight,
//! design eye, and the pinhole/no-distortion model assumption) each carry an
//! explicit engineering allowance with a stated rationale — never zero, because
//! "declared exactly" in the sim world is still an assumption a real integration
//! would have to bound. The total is a conservative linear (worst-case, not
//! root-sum-square) sum, so a consumer that adds it to its own budget never
//! under-counts.

use super::geometry::PinholeIntrinsics;

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

/// The design-eye positional allowance expressed as an angular parallax bound at
/// the reference range.
pub const DESIGN_EYE_ALLOWANCE_RAD: f64 = DESIGN_EYE_ALLOWANCE_M / DESIGN_EYE_REFERENCE_RANGE_M;

/// The stored, irreducible alignment-error components: the intrinsic residual
/// budget and the declared engineering allowances. Everything else (the
/// pixel→angle factor and the totals) is derived, not stored.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlignmentAllowances {
    /// The intrinsic reprojection-error budget, in pixels. Must cover the
    /// measured recovery residual (`identity.residuals.max_px`); may exceed it
    /// as a conservative allowance.
    pub intrinsic_residual_px: f64,
    /// Distortion / model-mismatch allowance, in pixels. Declared, strictly
    /// positive.
    pub distortion_model_allowance_px: f64,
    /// Extrinsics rotation allowance, in radians. Declared, strictly positive.
    pub extrinsics_rotation_allowance_rad: f64,
    /// Boresight allowance, in radians. Declared, strictly positive.
    pub boresight_allowance_rad: f64,
    /// Design-eye parallax allowance, in radians. Declared, strictly positive.
    pub design_eye_allowance_rad: f64,
}

impl AlignmentAllowances {
    /// The published simulator allowances for a given intrinsic residual budget:
    /// the declared constants plus the supplied residual.
    #[must_use]
    pub const fn sim_defaults(intrinsic_residual_px: f64) -> Self {
        Self {
            intrinsic_residual_px,
            distortion_model_allowance_px: DISTORTION_MODEL_ALLOWANCE_PX,
            extrinsics_rotation_allowance_rad: EXTRINSICS_ROTATION_ALLOWANCE_RAD,
            boresight_allowance_rad: BORESIGHT_ALLOWANCE_RAD,
            design_eye_allowance_rad: DESIGN_EYE_ALLOWANCE_RAD,
        }
    }
}

/// The calibration's contribution to the conservative conformal alignment error
/// budget. Every field here is **derived** from [`AlignmentAllowances`] and the
/// focal lengths; nothing here is stored.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlignmentErrorBudget {
    /// Intrinsic reprojection-error budget, in pixels (from the allowances).
    pub intrinsic_residual_px: f64,
    /// Distortion / model-mismatch allowance, in pixels (from the allowances).
    pub distortion_model_allowance_px: f64,
    /// Extrinsics rotation allowance, in radians (from the allowances).
    pub extrinsics_rotation_allowance_rad: f64,
    /// Boresight allowance, in radians (from the allowances).
    pub boresight_allowance_rad: f64,
    /// Design-eye parallax allowance, in radians (from the allowances).
    pub design_eye_allowance_rad: f64,
    /// Derived pixel-to-angle conversion, `1 / min(focal_x, focal_y)`.
    pub radians_per_pixel: f64,
    /// Derived total pixel bound: intrinsic residual plus the distortion
    /// allowance.
    pub total_pixel_bound_px: f64,
    /// Derived total angular bound, in radians: the pixel bound converted to
    /// angle plus the declared angular allowances, summed worst-case. The single
    /// number a downstream budget composes.
    pub total_angular_bound_rad: f64,
}

/// Derives the pixel-to-angle factor from the intrinsics: the smaller focal
/// length (the larger angle per pixel), for conservatism.
#[must_use]
pub fn radians_per_pixel(intrinsics: &PinholeIntrinsics) -> f64 {
    1.0 / intrinsics.focal_x_px.min(intrinsics.focal_y_px)
}

/// Derives the full alignment error budget from the stored allowances and the
/// camera's intrinsics. All totals are computed here; none are stored.
#[must_use]
pub fn derive_budget(
    intrinsics: &PinholeIntrinsics,
    allowances: &AlignmentAllowances,
) -> AlignmentErrorBudget {
    let radians_per_pixel = radians_per_pixel(intrinsics);
    let total_pixel_bound_px =
        allowances.intrinsic_residual_px + allowances.distortion_model_allowance_px;
    let total_angular_bound_rad = total_pixel_bound_px * radians_per_pixel
        + allowances.extrinsics_rotation_allowance_rad
        + allowances.boresight_allowance_rad
        + allowances.design_eye_allowance_rad;
    AlignmentErrorBudget {
        intrinsic_residual_px: allowances.intrinsic_residual_px,
        distortion_model_allowance_px: allowances.distortion_model_allowance_px,
        extrinsics_rotation_allowance_rad: allowances.extrinsics_rotation_allowance_rad,
        boresight_allowance_rad: allowances.boresight_allowance_rad,
        design_eye_allowance_rad: allowances.design_eye_allowance_rad,
        radians_per_pixel,
        total_pixel_bound_px,
        total_angular_bound_rad,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{derive_budget, radians_per_pixel};
    use crate::calibration::sim_fpv_calibration;

    #[test]
    fn derived_budget_totals_match_the_component_sums() {
        let cal = sim_fpv_calibration();
        let b = cal.budget();
        assert!((b.radians_per_pixel - radians_per_pixel(&cal.geometry.intrinsics)).abs() < 1e-15);
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
    fn budget_derives_the_same_from_allowances() {
        let cal = sim_fpv_calibration();
        let derived = derive_budget(&cal.geometry.intrinsics, &cal.allowances);
        assert_eq!(derived, cal.budget());
    }
}
