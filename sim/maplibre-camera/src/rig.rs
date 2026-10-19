//! The vehicle's cameras: their images, lenses, and mounts.
//!
//! Each camera is an undistorted pinhole with square pixels and its
//! principal point at the image centre, the same model the simulator's
//! published calibrations use (ADR-0021). A zoom detent changes only the
//! focal length.

use maplibre::render::camera::EyeFrustum;

/// Producer camera id of the forward camera fixed to the body.
pub const FPV_CAMERA: u32 = 0;
/// Producer camera id of the gimbal payload camera.
pub const GIMBAL_CAMERA: u32 = 2;

/// Near clip distance, metres: close enough to keep the ground in view
/// from a vehicle standing on it.
const NEAR_M: f64 = 0.3;
/// Far clip distance, metres: past any horizon a low-flying vehicle sees.
const FAR_M: f64 = 400_000.0;

/// Calibration ids of this producer: `0x4D4C` ("ML"), the camera id, and
/// the detent counted from one, so no id is zero.
const CALIBRATION_ID_BASE: u32 = 0x4D4C_0000;

/// How a camera is pointed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mount {
    /// Fixed to the body, pitched by the angle given, radians, positive up.
    Body {
        /// Mount pitch, radians, positive up.
        pitch_rad: f64,
    },
    /// On a horizon-stabilized gimbal that pans about the body heading.
    Gimbal,
}

/// One camera of the vehicle.
#[derive(Debug, Clone, PartialEq)]
pub struct CameraSpec {
    /// Producer camera id, which routes the frames to a video source.
    pub camera_id: u32,
    /// Image width, pixels.
    pub width: u32,
    /// Image height, pixels.
    pub height: u32,
    /// Horizontal field of view of each zoom detent, radians, widest first.
    pub detents_hfov_rad: Vec<f64>,
    /// Lens position in the body FRD frame, metres.
    pub mount_body_m: [f64; 3],
    /// How the camera is pointed.
    pub mount: Mount,
}

/// Pinhole intrinsics of one detent, pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Intrinsics {
    /// Focal length, pixels, equal on both axes.
    pub focal_px: f64,
    /// Principal point column.
    pub principal_x_px: f64,
    /// Principal point row.
    pub principal_y_px: f64,
}

impl CameraSpec {
    /// The forward camera: 90 degrees wide, 10 degrees nose down, at the nose.
    #[must_use]
    pub fn fpv((width, height): (u32, u32)) -> Self {
        Self {
            camera_id: FPV_CAMERA,
            width,
            height,
            detents_hfov_rad: vec![90_f64.to_radians()],
            mount_body_m: [0.15, 0.0, 0.05],
            mount: Mount::Body {
                pitch_rad: (-10_f64).to_radians(),
            },
        }
    }

    /// The gimbal payload camera under the body, with four zoom detents.
    #[must_use]
    pub fn gimbal((width, height): (u32, u32)) -> Self {
        Self {
            camera_id: GIMBAL_CAMERA,
            width,
            height,
            detents_hfov_rad: [70.0_f64, 40.0, 20.0, 8.0]
                .iter()
                .map(|degrees| degrees.to_radians())
                .collect(),
            mount_body_m: [0.05, 0.0, 0.12],
            mount: Mount::Gimbal,
        }
    }

    /// The detent a command asks for, held to the detents this camera has.
    #[must_use]
    pub fn detent(&self, requested: u32) -> usize {
        let last = self.detents_hfov_rad.len().saturating_sub(1);
        usize::try_from(requested).map_or(last, |requested| requested.min(last))
    }

    /// The pinhole intrinsics of `detent`.
    #[must_use]
    pub fn intrinsics(&self, detent: usize) -> Intrinsics {
        let hfov = self.detents_hfov_rad.get(detent).copied().unwrap_or(1.0);
        let half_width = f64::from(self.width) / 2.0;
        Intrinsics {
            focal_px: half_width / (hfov / 2.0).tan(),
            principal_x_px: half_width,
            principal_y_px: f64::from(self.height) / 2.0,
        }
    }

    /// The render frustum whose image is exactly the pinhole image of
    /// `detent`, so a pixel of a frame is where the calibration projects it.
    #[must_use]
    pub fn frustum(&self, detent: usize) -> EyeFrustum {
        let lens = self.intrinsics(detent);
        EyeFrustum {
            left: lens.principal_x_px / lens.focal_px,
            right: (f64::from(self.width) - lens.principal_x_px) / lens.focal_px,
            top: lens.principal_y_px / lens.focal_px,
            bottom: (f64::from(self.height) - lens.principal_y_px) / lens.focal_px,
            near: NEAR_M,
            far: FAR_M,
        }
    }

    /// The published calibration id of `detent`.
    #[must_use]
    pub fn calibration_id(&self, detent: usize) -> u32 {
        let detent = u32::try_from(detent).map_or(0xFE, |detent| detent.min(0xFE));
        CALIBRATION_ID_BASE | (self.camera_id & 0xFF) << 8 | (detent + 1)
    }
}

#[cfg(test)]
mod tests;
