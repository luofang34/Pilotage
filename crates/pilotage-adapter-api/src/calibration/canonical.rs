//! The canonical byte serialization a calibration's content hash is taken
//! over, plus fail-closed verification.
//!
//! The canonical form is a fixed little-endian layout with no timestamps,
//! padding, or platform-dependent ordering, so the same calibration always
//! produces the same bytes and therefore the same hash. `f64` fields are their
//! IEEE-754 little-endian bit patterns, so the hash is bit-exact across Rust
//! and the browser verifier (`clients/web/calibration.js`), which mirrors this
//! layout. The recorded hash is stored alongside the artifact, never inside
//! its canonical bytes; [`verify`] recomputes and compares, so a mutated value
//! whose recorded hash was not also updated fails closed.

use sha2::{Digest, Sha256};

use super::CameraCalibration;
use super::identity::ValidityStatus;
use crate::calibration::error::CalibrationError;

/// The calibration schema version. Bumped when the canonical layout changes;
/// mirrored by `CALIBRATION_SCHEMA_VERSION` in the browser verifier.
pub const CALIBRATION_SCHEMA_VERSION: u16 = 1;

/// Units marker: `0` means lengths in meters, angles in radians, image
/// coordinates in pixels. Serialized explicitly so a future unit change is a
/// visible schema event, not a silent reinterpretation.
const UNITS_METERS_RADIANS_PIXELS: u8 = 0;

fn push_f64(out: &mut Vec<u8>, value: f64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_f64s(out: &mut Vec<u8>, values: &[f64]) {
    for value in values {
        push_f64(out, *value);
    }
}

fn write_header(out: &mut Vec<u8>, cal: &CameraCalibration) {
    let id = &cal.identity;
    out.extend_from_slice(&CALIBRATION_SCHEMA_VERSION.to_le_bytes());
    out.extend_from_slice(&id.calibration_id.0.to_le_bytes());
    out.extend_from_slice(&id.camera_id.to_le_bytes());
    out.extend_from_slice(&id.version.0.to_le_bytes());
    out.extend_from_slice(&id.tool_version.major.to_le_bytes());
    out.extend_from_slice(&id.tool_version.minor.to_le_bytes());
    out.extend_from_slice(&id.tool_version.patch.to_le_bytes());
    out.extend_from_slice(&id.effective.start_unix_ns.to_le_bytes());
    out.extend_from_slice(&id.effective.end_unix_ns.to_le_bytes());
    out.extend_from_slice(&(id.provenance as u32).to_le_bytes());
    out.push(id.status as u8);
    out.push(UNITS_METERS_RADIANS_PIXELS);
    out.push(cal.geometry.intrinsics.convention as u8);
    out.push(cal.geometry.extrinsics.from_frame.to_u8());
    out.push(cal.geometry.extrinsics.to_frame.to_u8());
}

fn write_geometry(out: &mut Vec<u8>, cal: &CameraCalibration) {
    let g = &cal.geometry;
    out.extend_from_slice(&g.viewport.width_px.to_le_bytes());
    out.extend_from_slice(&g.viewport.height_px.to_le_bytes());
    let i = &g.intrinsics;
    push_f64s(
        out,
        &[
            i.focal_x_px,
            i.focal_y_px,
            i.principal_x_px,
            i.principal_y_px,
            i.skew_px,
        ],
    );
    let d = &g.distortion;
    push_f64s(
        out,
        &[
            d.radial_k1,
            d.radial_k2,
            d.radial_k3,
            d.tangential_p1,
            d.tangential_p2,
        ],
    );
    push_f64s(out, &[g.fov.horizontal_rad, g.fov.vertical_rad]);
    push_f64s(out, &g.extrinsics.translation_body_m);
    push_f64s(out, &g.extrinsics.rotation_quat_wxyz);
    push_f64s(out, &g.design_eye.position_installation_m);
    push_f64s(out, &g.boresight.direction_camera);
    push_f64s(
        out,
        &[cal.identity.residuals.rms_px, cal.identity.residuals.max_px],
    );
}

fn write_budget(out: &mut Vec<u8>, cal: &CameraCalibration) {
    let b = &cal.budget;
    push_f64s(
        out,
        &[
            b.intrinsic_residual_px,
            b.distortion_model_allowance_px,
            b.extrinsics_rotation_allowance_rad,
            b.boresight_allowance_rad,
            b.design_eye_allowance_rad,
            b.radians_per_pixel,
            b.total_pixel_bound_px,
            b.total_angular_bound_rad,
        ],
    );
}

/// Serializes a calibration into its canonical byte form.
#[must_use]
pub fn to_canonical(cal: &CameraCalibration) -> Vec<u8> {
    let mut out = Vec::new();
    write_header(&mut out, cal);
    write_geometry(&mut out, cal);
    write_budget(&mut out, cal);
    out
}

/// The SHA-256 content hash of a calibration's canonical form.
#[must_use]
pub fn content_hash(cal: &CameraCalibration) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(to_canonical(cal));
    hasher.finalize().into()
}

/// Verifies a calibration for use at `now_unix_ns`, failing closed.
///
/// The checks, in order: the recomputed content hash matches `recorded_hash`;
/// every geometry, lifecycle, and budget invariant is semantically valid (so a
/// hash-consistent but malformed artifact is still rejected); the status is
/// [`ValidityStatus::Valid`]; and `now_unix_ns` is within the effective window.
/// A wrong-camera check is separate ([`verify_camera`]), since it depends on the
/// frame being displayed.
///
/// # Errors
///
/// Returns the first failing [`CalibrationError`].
pub fn verify(
    cal: &CameraCalibration,
    recorded_hash: [u8; 32],
    now_unix_ns: u64,
) -> Result<(), CalibrationError> {
    let computed = content_hash(cal);
    if computed != recorded_hash {
        return Err(CalibrationError::ContentHashMismatch {
            expected: recorded_hash,
            computed,
        });
    }
    super::validate::validate(cal)?;
    if cal.identity.status != ValidityStatus::Valid {
        return Err(CalibrationError::NotValid {
            status: cal.identity.status,
        });
    }
    if !cal.identity.effective.contains(now_unix_ns) {
        return Err(CalibrationError::Expired {
            now_unix_ns,
            start_unix_ns: cal.identity.effective.start_unix_ns,
            end_unix_ns: cal.identity.effective.end_unix_ns,
        });
    }
    Ok(())
}

/// Verifies a calibration applies to `frame_camera_id`, failing closed on a
/// mismatch (the frame came from a different camera than the calibration
/// describes).
///
/// # Errors
///
/// [`CalibrationError::WrongCamera`] on a camera-id mismatch.
pub fn verify_camera(
    cal: &CameraCalibration,
    frame_camera_id: u32,
) -> Result<(), CalibrationError> {
    if cal.identity.camera_id != frame_camera_id {
        return Err(CalibrationError::WrongCamera {
            expected: cal.identity.camera_id,
            actual: frame_camera_id,
        });
    }
    Ok(())
}
