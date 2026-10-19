//! The published calibration of each camera detent (ADR-0021).
//!
//! A detent's calibration is derived from the same intrinsics the render
//! frustum is built from, so a frame stamped with it is the image that
//! calibration projects. The fit residuals come from the shared synthetic
//! recovery tool. A gimbal camera's extrinsics are stated at its neutral
//! pointing (pan and tilt zero); its pointing at a frame's capture is the
//! gimbal state the vehicle reports. SIM / NOT FOR FLIGHT.

use std::path::{Path, PathBuf};

use base64::Engine;
use cgmath::{Matrix3, Quaternion};
use indicate_frames::FrameId;
use pilotage_calibration_id::CalibrationId;
use pilotage_camera_calibration::{
    AlignmentAllowances, BodyToCameraExtrinsics, Boresight, BrownConradyDistortion,
    CalibrationIdentity, CalibrationVersion, CameraCalibration, CameraGeometry, DesignEye,
    EffectivePeriod, OpticalConvention, PinholeIntrinsics, ProvenanceSource, Residuals,
    ToolVersion, ValidityStatus, Viewport, recovery_report, to_canonical,
};

use crate::error::CameraError;
use crate::rig::{CameraSpec, Mount};

/// Effective window start: 2020-01-01T00:00:00Z, Unix nanoseconds.
const EFFECTIVE_START_NS: u64 = 1_577_836_800_000_000_000;
/// Effective window end: 2035-01-01T00:00:00Z, Unix nanoseconds.
const EFFECTIVE_END_NS: u64 = 2_051_222_400_000_000_000;

/// Camera optical (OpenCV: x right, y down, z forward) from the FRD axes of
/// the camera's own look direction.
fn optical_from_frd() -> Matrix3<f64> {
    // Columns: FRD forward is optical z, right is optical x, down is
    // optical y.
    Matrix3::new(0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0)
}

/// Body FRD to camera optical rotation, as the calibration stores it.
fn body_to_optical(camera: &CameraSpec) -> [f64; 4] {
    let pitch = match camera.mount {
        Mount::Body { pitch_rad } => pitch_rad,
        Mount::Gimbal => 0.0,
    };
    let (sin, cos) = pitch.sin_cos();
    // Body FRD into the pitched camera's FRD: the inverse of a nose-up
    // rotation about right by `pitch`.
    let frd_from_body = Matrix3::new(cos, 0.0, sin, 0.0, 1.0, 0.0, -sin, 0.0, cos);
    let rotation = Quaternion::from(optical_from_frd() * frd_from_body);
    // One hemisphere, so the stored form of a rotation is unique.
    let rotation = if rotation.s < 0.0 {
        -rotation
    } else {
        rotation
    };
    [rotation.s, rotation.v.x, rotation.v.y, rotation.v.z]
}

/// The calibration of `camera` at `detent`.
///
/// # Errors
///
/// [`CameraError::Calibration`] when the derived calibration fails the
/// shared validator.
pub fn calibration(camera: &CameraSpec, detent: usize) -> Result<CameraCalibration, CameraError> {
    let lens = camera.intrinsics(detent);
    let intrinsics = PinholeIntrinsics {
        focal_x_px: lens.focal_px,
        focal_y_px: lens.focal_px,
        principal_x_px: lens.principal_x_px,
        principal_y_px: lens.principal_y_px,
        skew_px: 0.0,
        convention: OpticalConvention::OpenCv,
    };
    let report = recovery_report(&intrinsics);
    let calibration = CameraCalibration {
        geometry: CameraGeometry {
            intrinsics,
            distortion: BrownConradyDistortion::NONE,
            viewport: Viewport {
                width_px: camera.width,
                height_px: camera.height,
            },
            extrinsics: BodyToCameraExtrinsics {
                translation_body_m: camera.mount_body_m,
                rotation_quat_wxyz: body_to_optical(camera),
                from_frame: FrameId::Body,
                to_frame: FrameId::Installation,
            },
            design_eye: DesignEye {
                position_installation_m: [0.0, 0.0, 0.0],
            },
            boresight: Boresight {
                direction_camera: [0.0, 0.0, 1.0],
            },
        },
        identity: CalibrationIdentity {
            calibration_id: CalibrationId(camera.calibration_id(detent)),
            camera_id: camera.camera_id,
            version: CalibrationVersion(1),
            tool_version: ToolVersion {
                major: 1,
                minor: 0,
                patch: 0,
            },
            effective: EffectivePeriod {
                start_unix_ns: EFFECTIVE_START_NS,
                end_unix_ns: EFFECTIVE_END_NS,
            },
            provenance: ProvenanceSource::SimSyntheticTool,
            residuals: Residuals {
                rms_px: report.residual_rms_px,
                max_px: report.residual_max_px,
            },
            status: ValidityStatus::Valid,
        },
        allowances: AlignmentAllowances::sim_defaults(report.residual_max_px),
    };
    calibration
        .validate()
        .map_err(|source| CameraError::Calibration {
            camera_id: camera.camera_id,
            detent,
            source,
        })?;
    Ok(calibration)
}

/// The artifact text of a calibration: its canonical bytes and their
/// recorded SHA-256, in the layout the browser registry reads.
#[must_use]
pub fn artifact_json(calibration: &CameraCalibration) -> String {
    let hash: String = calibration
        .content_hash()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let document = serde_json::json!({
        "description": format!(
            "SIMULATOR camera {} calibration {:#010x} (ADR-0021). The maplibre-rs world-model \
             camera producer derives it from its render frustum. SIM / NOT FOR FLIGHT.",
            calibration.identity.camera_id, calibration.identity.calibration_id.0
        ),
        "canonicalBase64": base64::engine::general_purpose::STANDARD
            .encode(to_canonical(calibration)),
        "recordedHashHex": hash,
    });
    format!("{document:#}\n")
}

/// Writes one artifact per detent of every camera into `dir`, named by
/// calibration id, and returns their paths.
///
/// # Errors
///
/// [`CameraError::Calibration`] for an invalid calibration and
/// [`CameraError::CalibrationWrite`] when a file cannot be written.
pub fn write_artifacts(dir: &Path, cameras: &[CameraSpec]) -> Result<Vec<PathBuf>, CameraError> {
    std::fs::create_dir_all(dir).map_err(|source| CameraError::CalibrationWrite {
        path: dir.to_owned(),
        source,
    })?;
    let mut written = Vec::new();
    for camera in cameras {
        for detent in 0..camera.detents_hfov_rad.len() {
            let calibration = calibration(camera, detent)?;
            let path = dir.join(format!(
                "calibration-{:08x}.json",
                calibration.identity.calibration_id.0
            ));
            std::fs::write(&path, artifact_json(&calibration)).map_err(|source| {
                CameraError::CalibrationWrite {
                    path: path.clone(),
                    source,
                }
            })?;
            written.push(path);
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests;
