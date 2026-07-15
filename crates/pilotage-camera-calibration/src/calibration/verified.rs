//! The verified camera model: the resolved geometry of a hash-verified
//! calibration artifact, with no public constructor.
//!
//! A [`VerifiedCameraModel`] can only be obtained through
//! [`super::CameraCalibration::verified_camera_model`], which recomputes and
//! checks the artifact's content hash before minting. The fields are private and
//! the constructor is crate-private, so no external crate can fabricate one:
//! holding a `VerifiedCameraModel` is proof the artifact was authenticated. A
//! conformal projector consumes the resolved geometry through the accessors and
//! reconstructs its own calibration reference from [`Self::calibration_id`] and
//! [`Self::content_hash`], so this crate does not depend on the projector.

use pilotage_calibration_id::CalibrationId;
use pilotage_frames::Quat;

/// The resolved geometry of a verified calibration: its identity and content
/// hash, the body→camera rotation, the field-of-view half-tangents, and the
/// published alignment bound. Obtainable only via
/// [`super::CameraCalibration::verified_camera_model`].
///
/// The fields are private and the constructor is crate-private, so an external
/// crate cannot fabricate one — a field-wise literal does not compile:
///
/// ```compile_fail
/// use pilotage_camera_calibration::VerifiedCameraModel;
/// // No public constructor and private fields: the only source is a genuine
/// // hash-verify via CameraCalibration::verified_camera_model.
/// let _ = VerifiedCameraModel {
///     calibration_id: todo!(),
///     content_hash: todo!(),
///     body_to_camera: todo!(),
///     half_fov_x_tan: todo!(),
///     half_fov_y_tan: todo!(),
///     alignment_bound_rad: todo!(),
/// };
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VerifiedCameraModel {
    calibration_id: CalibrationId,
    content_hash: [u8; 32],
    body_to_camera: Quat,
    half_fov_x_tan: f64,
    half_fov_y_tan: f64,
    alignment_bound_rad: f64,
}

impl VerifiedCameraModel {
    /// Mints a verified camera model. **Crate-private**: the only caller is the
    /// hash-verifying [`super::CameraCalibration::verified_camera_model`], so a
    /// value of this type cannot exist without a genuine content-hash
    /// verification of the artifact it describes.
    pub(crate) fn new(
        calibration_id: CalibrationId,
        content_hash: [u8; 32],
        body_to_camera: Quat,
        half_fov_tangents: (f64, f64),
        alignment_bound_rad: f64,
    ) -> Self {
        Self {
            calibration_id,
            content_hash,
            body_to_camera,
            half_fov_x_tan: half_fov_tangents.0,
            half_fov_y_tan: half_fov_tangents.1,
            alignment_bound_rad,
        }
    }

    /// The verified calibration identity.
    #[must_use]
    pub fn calibration_id(&self) -> CalibrationId {
        self.calibration_id
    }

    /// The verified content hash the identity commits to.
    #[must_use]
    pub fn content_hash(&self) -> [u8; 32] {
        self.content_hash
    }

    /// Body → camera optical-frame rotation (the verified extrinsics).
    #[must_use]
    pub fn body_to_camera(&self) -> Quat {
        self.body_to_camera
    }

    /// Tangents of the half horizontal and half vertical fields of view,
    /// `((width/2)/focal_x, (height/2)/focal_y)`.
    #[must_use]
    pub fn half_fov_tangents(&self) -> (f64, f64) {
        (self.half_fov_x_tan, self.half_fov_y_tan)
    }

    /// The published static angular alignment bound, radians.
    #[must_use]
    pub fn alignment_bound_rad(&self) -> f64 {
        self.alignment_bound_rad
    }
}
