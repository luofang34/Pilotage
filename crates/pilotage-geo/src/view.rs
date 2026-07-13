//! Viewport, projection, camera pose, and the derived field of view.
//!
//! Aligned with the SVS-01 sibling calibration contract (ADR-0021): the field
//! of view is **derived** from the viewport and focal lengths, never stored;
//! the optical convention is OpenCV; and the camera pose names its frames with
//! the `pilotage-frames` vocabulary (`Body` → `Installation`). Nothing here
//! renders — it declares the view a renderer must honor.

use pilotage_frames::{FrameId, Quat};

use crate::error::GeoError;

/// The optical coordinate convention intrinsics and the boresight are expressed
/// in. Explicit so a projection is never read against a guessed axis layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OpticalConvention {
    /// OpenCV: `+Z` along the optical axis, `+X` right, `+Y` down.
    OpenCv = 0,
}

impl OpticalConvention {
    /// Wire encoding.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }
    /// Decodes the wire byte, or `None` for an unknown value.
    #[must_use]
    pub const fn from_u8(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::OpenCv),
            _ => None,
        }
    }
}

/// The image sensor viewport, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    /// Image width, pixels.
    pub width_px: u32,
    /// Image height, pixels.
    pub height_px: u32,
}

/// Angular field of view, in radians. Always a **derived** result, never a
/// stored field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldOfView {
    /// Horizontal field of view, radians.
    pub horizontal_rad: f64,
    /// Vertical field of view, radians.
    pub vertical_rad: f64,
}

/// The projection a renderer must apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ProjectionKind {
    /// Perspective projection.
    Perspective = 0,
    /// Orthographic projection.
    Orthographic = 1,
}

impl ProjectionKind {
    /// Wire encoding.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }
    /// Decodes the wire byte, or `None` for an unknown value.
    #[must_use]
    pub const fn from_u8(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Perspective),
            1 => Some(Self::Orthographic),
            _ => None,
        }
    }
}

/// How a renderer samples terrain/tiles under minification (distant detail).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MinificationPolicy {
    /// Nearest-sample (no filtering).
    Nearest = 0,
    /// Bilinear within one level.
    Bilinear = 1,
    /// Trilinear across mip levels.
    Trilinear = 2,
}

impl MinificationPolicy {
    /// Wire encoding.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }
    /// Decodes the wire byte, or `None` for an unknown value.
    #[must_use]
    pub const fn from_u8(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Nearest),
            1 => Some(Self::Bilinear),
            2 => Some(Self::Trilinear),
            _ => None,
        }
    }
}

/// The near/far clip policy, in meters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NearFarPolicy {
    /// Near clip distance, meters (must be `> 0`).
    pub near_m: f64,
    /// Far clip distance, meters (must be `> near`).
    pub far_m: f64,
}

impl NearFarPolicy {
    /// Whether a depth (meters along the optical axis) is within the clip
    /// range, inclusive of both planes — the projection-boundary predicate.
    #[must_use]
    pub fn contains_depth(&self, depth_m: f64) -> bool {
        depth_m >= self.near_m && depth_m <= self.far_m
    }
}

/// The camera pose: where the camera sits and how it is oriented, with both
/// frames named (`Body` → `Installation`, the sensor mount).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraPose {
    /// Camera optical-frame origin in the `from_frame`, meters.
    pub translation_m: [f64; 3],
    /// Rotation from `from_frame` directions to the camera optical frame.
    pub attitude: Quat,
    /// The frame the translation is in and the rotation maps from.
    pub from_frame: FrameId,
    /// The frame the rotation maps into (the sensor/installation mount).
    pub to_frame: FrameId,
}

/// The complete projection view: viewport, focal lengths, projection, clip
/// policy, minification, convention, and camera pose. The field of view is
/// derived from the viewport and focal lengths, never stored.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectionView {
    /// Image viewport.
    pub viewport: Viewport,
    /// Focal length x, pixels (`> 0`).
    pub focal_x_px: f64,
    /// Focal length y, pixels (`> 0`).
    pub focal_y_px: f64,
    /// Projection kind.
    pub projection: ProjectionKind,
    /// Near/far clip policy.
    pub near_far: NearFarPolicy,
    /// Minification sampling policy.
    pub minification: MinificationPolicy,
    /// Optical convention.
    pub convention: OpticalConvention,
    /// Camera pose.
    pub camera: CameraPose,
}

impl ProjectionView {
    /// Derives the angular field of view from the viewport and focal lengths:
    /// `fov = 2·atan((size/2)/focal)` per axis.
    #[must_use]
    pub fn field_of_view(&self) -> FieldOfView {
        let half_w = f64::from(self.viewport.width_px) / 2.0;
        let half_h = f64::from(self.viewport.height_px) / 2.0;
        FieldOfView {
            horizontal_rad: 2.0 * libm::atan(half_w / self.focal_x_px),
            vertical_rad: 2.0 * libm::atan(half_h / self.focal_y_px),
        }
    }

    /// Validates the view, failing closed on a degenerate viewport, a
    /// non-positive focal, an invalid clip policy, a non-finite camera
    /// translation, a non-unit camera attitude, or wrong pose frames.
    ///
    /// # Errors
    ///
    /// A [`GeoError`] describing the first violation.
    pub fn validate(&self) -> Result<(), GeoError> {
        if self.viewport.width_px == 0 || self.viewport.height_px == 0 {
            return Err(GeoError::InvalidViewport {
                width_px: self.viewport.width_px,
                height_px: self.viewport.height_px,
            });
        }
        if !(self.focal_x_px.is_finite() && self.focal_y_px.is_finite())
            || self.focal_x_px <= 0.0
            || self.focal_y_px <= 0.0
        {
            return Err(GeoError::NonPositiveFocal {
                focal_x_px: self.focal_x_px,
                focal_y_px: self.focal_y_px,
            });
        }
        if !(self.near_far.near_m.is_finite() && self.near_far.far_m.is_finite())
            || self.near_far.near_m <= 0.0
            || self.near_far.far_m <= self.near_far.near_m
        {
            return Err(GeoError::InvalidNearFar {
                near_m: self.near_far.near_m,
                far_m: self.near_far.far_m,
            });
        }
        if !self.camera.translation_m.iter().all(|v| v.is_finite()) {
            return Err(GeoError::NonFinite {
                field: "camera_translation_m",
            });
        }
        if self.camera.attitude.renormalized(1e-4).is_err() {
            return Err(GeoError::NonFinite {
                field: "camera_attitude_not_a_rotation",
            });
        }
        if self.camera.from_frame != FrameId::Body || self.camera.to_frame != FrameId::Installation
        {
            return Err(GeoError::NonFinite {
                field: "camera_pose_frames",
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
