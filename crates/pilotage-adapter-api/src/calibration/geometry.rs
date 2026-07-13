//! Geometry of a SIMULATED pinhole camera and its design-eye reference.
//!
//! This describes a synthetic camera and a design-eye point in a simulator, so
//! a conformal overlay can be projected consistently in SIM. It is **not** a
//! model of real head-up-display optics: it has no combiner, no collimation,
//! no eyebox, and no installation-alignment terms, and it must never be
//! described as optical HUD qualification. The [`super`] module documents that
//! distinction; the types here keep every unit and coordinate frame explicit
//! (`_px` pixels, `_m` meters, `_rad` radians) so nothing is implied.

use pilotage_frames::FrameId;

/// The optical coordinate convention a camera's projection is expressed in.
///
/// Explicit so intrinsics and the boresight cannot be read against a guessed
/// axis layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OpticalConvention {
    /// OpenCV/computer-vision convention: `+Z` along the optical axis (into the
    /// scene), `+X` right, `+Y` down, origin at the principal point.
    OpenCv = 0,
}

/// Pinhole intrinsics, all in pixels. No unit is implied: each field names it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PinholeIntrinsics {
    /// Focal length along the image `x` axis, in pixels.
    pub focal_x_px: f64,
    /// Focal length along the image `y` axis, in pixels.
    pub focal_y_px: f64,
    /// Principal point `x` (optical-axis image column), in pixels.
    pub principal_x_px: f64,
    /// Principal point `y` (optical-axis image row), in pixels.
    pub principal_y_px: f64,
    /// Axis-skew coefficient, in pixels; `0.0` for square, unskewed pixels.
    pub skew_px: f64,
    /// The optical convention `x`/`y`/`z` are expressed in.
    pub convention: OpticalConvention,
}

/// Brown-Conrady lens distortion coefficients, all dimensionless. An ideal
/// pinhole (the simulator's default) uses [`Self::NONE`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrownConradyDistortion {
    /// First radial coefficient.
    pub radial_k1: f64,
    /// Second radial coefficient.
    pub radial_k2: f64,
    /// Third radial coefficient.
    pub radial_k3: f64,
    /// First tangential coefficient.
    pub tangential_p1: f64,
    /// Second tangential coefficient.
    pub tangential_p2: f64,
}

impl BrownConradyDistortion {
    /// No distortion: an ideal pinhole.
    pub const NONE: Self = Self {
        radial_k1: 0.0,
        radial_k2: 0.0,
        radial_k3: 0.0,
        tangential_p1: 0.0,
        tangential_p2: 0.0,
    };

    /// Whether all coefficients are zero (an ideal pinhole).
    #[must_use]
    pub fn is_ideal(&self) -> bool {
        *self == Self::NONE
    }
}

/// The image sensor viewport, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    /// Image width, in pixels.
    pub width_px: u32,
    /// Image height, in pixels.
    pub height_px: u32,
}

/// Angular field of view, in radians.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldOfView {
    /// Horizontal field of view, in radians.
    pub horizontal_rad: f64,
    /// Vertical field of view, in radians.
    pub vertical_rad: f64,
}

/// Rigid body-to-camera extrinsics: where the camera sits relative to the
/// vehicle body, with both frames named ([`FrameId::Body`] →
/// [`FrameId::Installation`], the sensor mount) so composition can never
/// silently relabel geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyToCameraExtrinsics {
    /// Camera optical-frame origin expressed in the body frame, in meters.
    pub translation_body_m: [f64; 3],
    /// Rotation quaternion `(w, x, y, z)` taking body-frame directions to the
    /// camera optical frame.
    pub rotation_quat_wxyz: [f64; 4],
    /// The frame the translation is expressed in and the rotation maps from.
    pub from_frame: FrameId,
    /// The frame the rotation maps into (the sensor/installation mount).
    pub to_frame: FrameId,
}

/// The SIMULATED design-eye reference point, expressed in the installation
/// frame, in meters. This is a synthetic reference for SIM projection, not a
/// real HUD eyebox or design eye position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DesignEye {
    /// Design-eye position in the installation frame, in meters.
    pub position_installation_m: [f64; 3],
}

/// The boresight direction as a unit vector in the camera optical frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Boresight {
    /// Boresight direction (unit vector) in the camera optical frame.
    pub direction_camera: [f64; 3],
}

/// The complete geometry of one simulated camera: intrinsics, distortion,
/// viewport, field of view, extrinsics, design eye, and boresight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraGeometry {
    /// Pinhole intrinsics.
    pub intrinsics: PinholeIntrinsics,
    /// Lens distortion.
    pub distortion: BrownConradyDistortion,
    /// Image viewport.
    pub viewport: Viewport,
    /// Angular field of view.
    pub fov: FieldOfView,
    /// Body-to-camera extrinsics.
    pub extrinsics: BodyToCameraExtrinsics,
    /// Simulated design-eye reference.
    pub design_eye: DesignEye,
    /// Boresight direction.
    pub boresight: Boresight,
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{BrownConradyDistortion, OpticalConvention};

    #[test]
    fn distortion_none_is_ideal() {
        assert!(BrownConradyDistortion::NONE.is_ideal());
        let distorted = BrownConradyDistortion {
            radial_k1: 0.1,
            ..BrownConradyDistortion::NONE
        };
        assert!(!distorted.is_ideal());
    }

    #[test]
    fn optical_convention_is_opencv() {
        assert_eq!(OpticalConvention::OpenCv as u8, 0);
    }
}
