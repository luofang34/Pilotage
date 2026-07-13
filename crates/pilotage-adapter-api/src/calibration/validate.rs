//! Typed semantic validation of a calibration's geometry, lifecycle, and
//! alignment budget (ADR-0021).
//!
//! Hash integrity authenticates that an artifact's bytes are what the publisher
//! recorded; it says nothing about whether those bytes describe a usable
//! camera. A hash-consistent artifact can still carry a NaN focal length, a
//! zero viewport, a non-unit rotation, or an inverted effective window. This
//! module checks every such invariant and fails closed with a distinct typed
//! reason, never clamping or repairing. It runs from [`super::verify`] and is
//! mirrored by the browser admission path in `clients/web/calibration.js`.

use pilotage_frames::FrameId;

use super::CameraCalibration;
use super::budget::AlignmentErrorBudget;
use super::error::CalibrationError;
use super::geometry::CameraGeometry;
use super::identity::CalibrationIdentity;

/// Tolerance on a unit-norm check for the rotation quaternion and boresight.
const UNIT_NORM_TOLERANCE: f64 = 1e-6;
/// Tolerance on the alignment-budget internal-consistency check.
const BUDGET_CONSISTENCY_TOLERANCE: f64 = 1e-9;

fn require_finite(field: &'static str, value: f64) -> Result<(), CalibrationError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(CalibrationError::NonFinite { field })
    }
}

fn require_all_finite(fields: &[(&'static str, f64)]) -> Result<(), CalibrationError> {
    for (name, value) in fields {
        require_finite(name, *value)?;
    }
    Ok(())
}

fn validate_intrinsics_viewport(geometry: &CameraGeometry) -> Result<(), CalibrationError> {
    let i = &geometry.intrinsics;
    require_all_finite(&[
        ("focal_x_px", i.focal_x_px),
        ("focal_y_px", i.focal_y_px),
        ("principal_x_px", i.principal_x_px),
        ("principal_y_px", i.principal_y_px),
        ("skew_px", i.skew_px),
    ])?;
    let v = &geometry.viewport;
    if v.width_px == 0 || v.height_px == 0 {
        return Err(CalibrationError::InvalidViewport {
            width_px: v.width_px,
            height_px: v.height_px,
        });
    }
    if i.focal_x_px <= 0.0 || i.focal_y_px <= 0.0 {
        return Err(CalibrationError::NonPositiveFocal {
            focal_x_px: i.focal_x_px,
            focal_y_px: i.focal_y_px,
        });
    }
    if i.principal_x_px < 0.0
        || i.principal_x_px > f64::from(v.width_px)
        || i.principal_y_px < 0.0
        || i.principal_y_px > f64::from(v.height_px)
    {
        return Err(CalibrationError::PrincipalPointOutOfBounds {
            principal_x_px: i.principal_x_px,
            principal_y_px: i.principal_y_px,
            width_px: v.width_px,
            height_px: v.height_px,
        });
    }
    Ok(())
}

fn validate_fov(geometry: &CameraGeometry) -> Result<(), CalibrationError> {
    let f = &geometry.fov;
    require_all_finite(&[
        ("fov_horizontal_rad", f.horizontal_rad),
        ("fov_vertical_rad", f.vertical_rad),
    ])?;
    if f.horizontal_rad <= 0.0
        || f.horizontal_rad >= core::f64::consts::PI
        || f.vertical_rad <= 0.0
        || f.vertical_rad >= core::f64::consts::PI
    {
        return Err(CalibrationError::InvalidFieldOfView {
            horizontal_rad: f.horizontal_rad,
            vertical_rad: f.vertical_rad,
        });
    }
    Ok(())
}

fn norm(components: &[f64]) -> f64 {
    components.iter().map(|c| c * c).sum::<f64>().sqrt()
}

fn validate_extrinsics_boresight(geometry: &CameraGeometry) -> Result<(), CalibrationError> {
    let d = &geometry.distortion;
    require_all_finite(&[
        ("radial_k1", d.radial_k1),
        ("radial_k2", d.radial_k2),
        ("radial_k3", d.radial_k3),
        ("tangential_p1", d.tangential_p1),
        ("tangential_p2", d.tangential_p2),
    ])?;
    let e = &geometry.extrinsics;
    require_all_finite(&[
        ("translation_x_m", e.translation_body_m[0]),
        ("translation_y_m", e.translation_body_m[1]),
        ("translation_z_m", e.translation_body_m[2]),
        ("quat_w", e.rotation_quat_wxyz[0]),
        ("quat_x", e.rotation_quat_wxyz[1]),
        ("quat_y", e.rotation_quat_wxyz[2]),
        ("quat_z", e.rotation_quat_wxyz[3]),
        (
            "design_eye_x_m",
            geometry.design_eye.position_installation_m[0],
        ),
        (
            "design_eye_y_m",
            geometry.design_eye.position_installation_m[1],
        ),
        (
            "design_eye_z_m",
            geometry.design_eye.position_installation_m[2],
        ),
        ("boresight_x", geometry.boresight.direction_camera[0]),
        ("boresight_y", geometry.boresight.direction_camera[1]),
        ("boresight_z", geometry.boresight.direction_camera[2]),
    ])?;
    if e.from_frame != FrameId::Body || e.to_frame != FrameId::Installation {
        return Err(CalibrationError::FrameMismatch {
            from_code: e.from_frame.to_u8(),
            to_code: e.to_frame.to_u8(),
        });
    }
    let quat_norm = norm(&e.rotation_quat_wxyz);
    if (quat_norm - 1.0).abs() > UNIT_NORM_TOLERANCE {
        return Err(CalibrationError::NonUnitQuaternion { norm: quat_norm });
    }
    let boresight_norm = norm(&geometry.boresight.direction_camera);
    if (boresight_norm - 1.0).abs() > UNIT_NORM_TOLERANCE {
        return Err(CalibrationError::NonUnitBoresight {
            norm: boresight_norm,
        });
    }
    Ok(())
}

fn validate_lifecycle(identity: &CalibrationIdentity) -> Result<(), CalibrationError> {
    let period = &identity.effective;
    if period.start_unix_ns >= period.end_unix_ns {
        return Err(CalibrationError::InvalidEffectivePeriod {
            start_unix_ns: period.start_unix_ns,
            end_unix_ns: period.end_unix_ns,
        });
    }
    let r = &identity.residuals;
    if !r.rms_px.is_finite()
        || !r.max_px.is_finite()
        || r.rms_px < 0.0
        || r.max_px < 0.0
        || r.rms_px > r.max_px
    {
        return Err(CalibrationError::InvalidResiduals {
            rms_px: r.rms_px,
            max_px: r.max_px,
        });
    }
    Ok(())
}

fn validate_budget(budget: &AlignmentErrorBudget) -> Result<(), CalibrationError> {
    let invalid = |reason| CalibrationError::InvalidAlignmentBudget { reason };
    let values = [
        budget.intrinsic_residual_px,
        budget.distortion_model_allowance_px,
        budget.extrinsics_rotation_allowance_rad,
        budget.boresight_allowance_rad,
        budget.design_eye_allowance_rad,
        budget.radians_per_pixel,
        budget.total_pixel_bound_px,
        budget.total_angular_bound_rad,
    ];
    if values.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Err(invalid("non-finite or negative bound"));
    }
    if budget.radians_per_pixel <= 0.0 {
        return Err(invalid("non-positive radians-per-pixel"));
    }
    let pixel = budget.intrinsic_residual_px + budget.distortion_model_allowance_px;
    if (budget.total_pixel_bound_px - pixel).abs() > BUDGET_CONSISTENCY_TOLERANCE {
        return Err(invalid(
            "total pixel bound is not the sum of its components",
        ));
    }
    let angular = budget.total_pixel_bound_px * budget.radians_per_pixel
        + budget.extrinsics_rotation_allowance_rad
        + budget.boresight_allowance_rad
        + budget.design_eye_allowance_rad;
    if (budget.total_angular_bound_rad - angular).abs() > BUDGET_CONSISTENCY_TOLERANCE {
        return Err(invalid(
            "total angular bound is not the sum of its components",
        ));
    }
    Ok(())
}

/// Validates every geometry, lifecycle, and budget invariant, failing closed
/// with the first violation.
///
/// # Errors
///
/// A distinct [`CalibrationError`] per invariant class.
pub fn validate(cal: &CameraCalibration) -> Result<(), CalibrationError> {
    validate_intrinsics_viewport(&cal.geometry)?;
    validate_fov(&cal.geometry)?;
    validate_extrinsics_boresight(&cal.geometry)?;
    validate_lifecycle(&cal.identity)?;
    validate_budget(&cal.budget)?;
    Ok(())
}
