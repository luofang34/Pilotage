//! The single published simulator calibration artifact (ADR-0021).
//!
//! This is the ONE source of the simulator's onboard-FPV camera calibration:
//! there is no scattered default FOV, principal point, or extrinsics anywhere
//! in the conformal path. Its geometry is pinned to the Gazebo world
//! (`sim/worlds/pilotage_yard.sdf`): a 320x240 image with a 1.396 rad
//! horizontal field of view, mounted on the vehicle body at `(1.1, 0, 0.3) m`.
//! It is a SIMULATED camera and a synthetic design eye — never real HUD optics.

use super::CameraCalibration;
use super::geometry::{
    BodyToCameraExtrinsics, Boresight, BrownConradyDistortion, CameraGeometry, DesignEye,
    FieldOfView, OpticalConvention, PinholeIntrinsics, Viewport,
};
use super::identity::{
    CalibrationId, CalibrationIdentity, CalibrationVersion, EffectivePeriod, ProvenanceSource,
    Residuals, ToolVersion, ValidityStatus,
};
use super::recovery::recovery_report;
use pilotage_frames::FrameId;

/// The FPV camera's physical id, matching the Gazebo bridge `camera_id` for the
/// onboard camera and the calibration id the artifact publishes.
pub const SIM_FPV_CAMERA_ID: u32 = 0;

/// The published calibration id for the simulator FPV camera. Non-zero, so it
/// is a published id (not [`CalibrationId::NONE`]).
pub const SIM_FPV_CALIBRATION_ID: u32 = 1;

/// FPV image width in pixels (matches the sim world).
const FPV_WIDTH_PX: u32 = 320;
/// FPV image height in pixels (matches the sim world).
const FPV_HEIGHT_PX: u32 = 240;
/// FPV horizontal field of view in radians (matches the sim world).
const FPV_HFOV_RAD: f64 = 1.396;

/// Effective window start: 2020-01-01T00:00:00Z, in Unix nanoseconds.
const EFFECTIVE_START_NS: u64 = 1_577_836_800_000_000_000;
/// Effective window end: 2035-01-01T00:00:00Z, in Unix nanoseconds.
const EFFECTIVE_END_NS: u64 = 2_051_222_400_000_000_000;

fn fpv_intrinsics() -> PinholeIntrinsics {
    // Square pixels, principal point at the image center; focal length from the
    // horizontal FOV: fx = (width/2) / tan(hfov/2).
    let half_w = f64::from(FPV_WIDTH_PX) / 2.0;
    let focal = half_w / (FPV_HFOV_RAD / 2.0).tan();
    PinholeIntrinsics {
        focal_x_px: focal,
        focal_y_px: focal,
        principal_x_px: half_w,
        principal_y_px: f64::from(FPV_HEIGHT_PX) / 2.0,
        skew_px: 0.0,
        convention: OpticalConvention::OpenCv,
    }
}

fn fpv_geometry() -> CameraGeometry {
    let intrinsics = fpv_intrinsics();
    let vertical_rad = 2.0 * (f64::from(FPV_HEIGHT_PX) / 2.0 / intrinsics.focal_y_px).atan();
    CameraGeometry {
        intrinsics,
        distortion: BrownConradyDistortion::NONE,
        viewport: Viewport {
            width_px: FPV_WIDTH_PX,
            height_px: FPV_HEIGHT_PX,
        },
        fov: FieldOfView {
            horizontal_rad: FPV_HFOV_RAD,
            vertical_rad,
        },
        extrinsics: BodyToCameraExtrinsics {
            translation_body_m: [1.1, 0.0, 0.3],
            // Body FRD (x forward, y right, z down) -> camera optical OpenCV
            // (x right, y down, z forward): forward->+Z, right->+X, down->+Y.
            rotation_quat_wxyz: [0.5, -0.5, -0.5, -0.5],
            from_frame: FrameId::Body,
            to_frame: FrameId::Installation,
        },
        // Simulated design eye coincident with the camera optical center; this
        // is a SIM reference, not a real HUD eyebox.
        design_eye: DesignEye {
            position_installation_m: [0.0, 0.0, 0.0],
        },
        boresight: Boresight {
            direction_camera: [0.0, 0.0, 1.0],
        },
    }
}

/// The published simulator FPV calibration. Its residuals are the deterministic
/// output of the synthetic-target recovery over its own intrinsics, so the
/// artifact records the fit it was produced by.
#[must_use]
pub fn sim_fpv_calibration() -> CameraCalibration {
    let geometry = fpv_geometry();
    let report = recovery_report(&geometry.intrinsics);
    let budget = super::budget::derive_budget(&geometry, report.residual_max_px);
    CameraCalibration {
        geometry,
        identity: CalibrationIdentity {
            calibration_id: CalibrationId(SIM_FPV_CALIBRATION_ID),
            camera_id: SIM_FPV_CAMERA_ID,
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
        budget,
    }
}

/// The recorded SHA-256 content hash of [`sim_fpv_calibration`], produced by the
/// build itself (see the `sim_fpv_hash_is_recorded` test, which recomputes it).
/// The browser artifact carries the same hash in hex.
pub const SIM_FPV_CALIBRATION_HASH: [u8; 32] = [
    0xa0, 0x6d, 0x80, 0x37, 0x88, 0x38, 0x11, 0xfc, 0xb7, 0xfc, 0xae, 0xe6, 0x18, 0x0d, 0xf1, 0xb6,
    0xed, 0x38, 0x72, 0xbc, 0x54, 0x2f, 0x25, 0xff, 0xe8, 0xe8, 0xd3, 0x27, 0xd1, 0xfa, 0xe4, 0x1c,
];
