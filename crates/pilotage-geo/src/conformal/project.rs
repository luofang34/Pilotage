//! Projecting the conformal cues — horizon, flight-path marker, and runway/path
//! cues — through the referenced camera model, in an explicit coordinate
//! reference.
//!
//! # Coordinate and velocity references (explicit)
//!
//! The camera optical frame follows the calibration's OpenCV convention: `+Z`
//! along the optical axis into the scene, `+X` right, `+Y` **down**, origin at
//! the principal point (the boresight). A direction `(X, Y, Z)` in that frame
//! with `Z > 0` projects to the **normalized image coordinate**
//! `(x, y) = (X/Z, Y/Z)` — the tangent of the angle off boresight, independent
//! of focal length. A consumer maps it to pixels with the resolved intrinsics:
//! `u = principal_x + focal_x · x`, `v = principal_y + focal_y · y`. Keeping the
//! projection focal-independent is deliberate: this crate references the
//! calibration but does not re-mint its intrinsics.
//!
//! The **horizon** is the locus of viewing rays perpendicular to the local
//! vertical (NED down). Its image is the line `a·x + b·y + c = 0` whose
//! coefficients are exactly the NED-down direction expressed in the camera frame;
//! the ground half-plane is the side where `a·x + b·y + c > 0` (the side the down
//! vector points).
//!
//! The **flight-path marker** sits at the direction of the aircraft's
//! **ground-referenced NED velocity** (not air-mass, not heading), so under a
//! crosswind it sits off the nose by the drift angle. It is drawn only when the
//! velocity direction is in front of the camera; behind or grazing the optical
//! axis it is removed.
//!
//! The **runway/path cues** are finite ground features (a runway outline, an
//! approach path, or a flight-path tunnel gate) supplied as offsets in the local
//! NED frame relative to the ownship, meters. Each is projected through the same
//! camera and clipped to the projection view's near/far policy: a point behind
//! the camera or outside the clip range is removed, and a point in front but
//! outside the field is marked off-scale.
//!
//! # Binding to one calibration
//!
//! [`ViewGeometry`] can only be **derived** from a [`VerifiedCameraModel`]
//! ([`ViewGeometry::derive`]) — the unforgeable verified model minted by
//! `pilotage-camera-calibration` only after a genuine content-hash verification.
//! [`ViewGeometry`]'s fields are crate-private, so a caller cannot hand-build one
//! that pairs a copied calibration identity with forged geometry; the derived
//! value embeds the [`CalibrationRef`] rebuilt from the verified model's identity
//! and content hash, so the extrinsics, field of view, and alignment bound it
//! carries provably belong to that one authenticated calibration.
//! [`crate::assess_conformal`] additionally refuses (fail-closed) a geometry
//! whose calibration does not match the [`crate::ProjectionView`]'s, so there is
//! no path to a registered scene without a genuinely verified calibration.

use pilotage_camera_calibration::VerifiedCameraModel;
use pilotage_frames::{Quat, ROTATION_NORM_TOLERANCE};

use super::policy::ConformalError;
use crate::view::{CalibrationRef, NearFarPolicy};

/// Speed below which the ground-referenced velocity has no well-defined
/// direction, so the flight-path marker is not drawn, meters/second.
const MIN_SPEED_MPS: f64 = 0.1;

/// Minimum camera-axis component for a direction to be treated as in front of
/// the camera; at or below it the projection is grazing/behind and is removed.
const FORWARD_EPS: f64 = 1e-6;

/// The horizon line in normalized image coordinates: the set of points `(x, y)`
/// with `a·x + b·y + c = 0`. The coefficients are the NED-down direction in the
/// camera optical frame, so the ground half-plane is `a·x + b·y + c > 0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HorizonLine {
    /// Coefficient of the normalized image `x`.
    pub a: f64,
    /// Coefficient of the normalized image `y`.
    pub b: f64,
    /// Constant term.
    pub c: f64,
}

/// A cue marked at a normalized image coordinate, with whether it falls inside
/// the referenced camera's field of view. A mark outside the field is the
/// off-scale indication: the consumer clamps it to the frame edge rather than
/// drawing it as if registered off-screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenMark {
    /// Normalized image `x` = `X_cam / Z_cam` (tangent of the horizontal angle
    /// off boresight).
    pub x: f64,
    /// Normalized image `y` = `Y_cam / Z_cam` (tangent of the vertical angle off
    /// boresight; positive is down).
    pub y: f64,
    /// Whether the mark lies within the field of view.
    pub within_fov: bool,
}

/// Validates resolved geometry components, failing closed: the calibration
/// reference must be well-formed, the body→camera quaternion must be a unit
/// rotation, each half-FOV tangent must be finite and strictly positive (a
/// half-angle in `(0, 90°)`, a full field under 180°), and the alignment bound
/// must be finite and non-negative.
fn validate_components(
    calibration: CalibrationRef,
    body_to_camera: Quat,
    half_fov_x_tan: f64,
    half_fov_y_tan: f64,
    alignment_bound_rad: f64,
) -> Result<(), ConformalError> {
    let bad = |field| Err(ConformalError::InvalidGeometry { field });
    if calibration.validate().is_err() {
        return bad("calibration");
    }
    if body_to_camera
        .renormalized(ROTATION_NORM_TOLERANCE)
        .is_err()
    {
        return bad("body_to_camera");
    }
    if !(half_fov_x_tan.is_finite() && half_fov_x_tan > 0.0) {
        return bad("half_fov_x_tan");
    }
    if !(half_fov_y_tan.is_finite() && half_fov_y_tan > 0.0) {
        return bad("half_fov_y_tan");
    }
    if !(alignment_bound_rad.is_finite() && alignment_bound_rad >= 0.0) {
        return bad("alignment_bound_rad");
    }
    Ok(())
}

/// The resolved render-time viewing geometry for one projection call: the
/// calibration it was derived from, the body→camera rotation, the field-of-view
/// half-extents, and the calibration's published alignment bound.
///
/// This is **not** a second camera model: it embeds the existing
/// [`CalibrationRef`] (identity and content hash only) and carries no intrinsics,
/// distortion, or lifecycle — those live in the calibration contract.
///
/// Its fields are crate-private and there is no public field-wise constructor:
/// the only way to obtain one is [`ViewGeometry::derive`] from a
/// [`VerifiedCameraModel`]. A calibration binding therefore cannot be forged by
/// copying a valid identity into hand-built geometry — that does not compile:
///
/// ```compile_fail
/// use pilotage_geo::ViewGeometry;
/// // The fields are private; a copied calibration identity cannot be paired with
/// // arbitrary geometry. Only `ViewGeometry::derive` from a verified calibration
/// // produces a value.
/// let _ = ViewGeometry {
///     calibration: todo!(),
///     body_to_camera: todo!(),
///     half_fov_x_tan: todo!(),
///     half_fov_y_tan: todo!(),
///     alignment_bound_rad: todo!(),
/// };
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewGeometry {
    /// The calibration this geometry was derived from (must equal the
    /// projection view's calibration).
    pub(crate) calibration: CalibrationRef,
    /// Rotation taking body-frame directions into the camera optical frame.
    pub(crate) body_to_camera: Quat,
    /// Tangent of the half horizontal field of view.
    pub(crate) half_fov_x_tan: f64,
    /// Tangent of the half vertical field of view.
    pub(crate) half_fov_y_tan: f64,
    /// The calibration's published static angular alignment bound, radians.
    pub(crate) alignment_bound_rad: f64,
}

impl ViewGeometry {
    /// Derives the resolved geometry from a [`VerifiedCameraModel`] — the only way
    /// to obtain a [`ViewGeometry`]. The `model` can itself only be minted by a
    /// genuine content-hash verification in `pilotage-camera-calibration`, so a
    /// derived geometry is authenticated by construction. Every field is read from
    /// the one model — the [`CalibrationRef`] is rebuilt from its identity and
    /// content hash — so the binding is a product of derivation, not a copyable
    /// claim, and the derived value is validated before it is returned.
    ///
    /// # Errors
    ///
    /// [`ConformalError::InvalidGeometry`] when the derived values are malformed
    /// (an incomplete reference, a non-unit rotation, a non-positive field of
    /// view, or a negative/non-finite bound).
    pub fn derive(model: &VerifiedCameraModel) -> Result<Self, ConformalError> {
        let (half_fov_x_tan, half_fov_y_tan) = model.half_fov_tangents();
        let geometry = Self {
            calibration: CalibrationRef {
                calibration_id: model.calibration_id(),
                content_hash: model.content_hash(),
            },
            body_to_camera: model.body_to_camera(),
            half_fov_x_tan,
            half_fov_y_tan,
            alignment_bound_rad: model.alignment_bound_rad(),
        };
        geometry.validate()?;
        Ok(geometry)
    }

    /// Validates the resolved geometry, failing closed: the calibration reference
    /// must be well-formed, the body→camera quaternion must be a unit rotation,
    /// each half-FOV tangent must be finite and strictly positive (a half-angle in
    /// `(0, 90°)`, a full field under 180°), and the alignment bound must be finite
    /// and non-negative.
    ///
    /// # Errors
    ///
    /// [`ConformalError::InvalidGeometry`] naming the offending field.
    pub fn validate(&self) -> Result<(), ConformalError> {
        validate_components(
            self.calibration,
            self.body_to_camera,
            self.half_fov_x_tan,
            self.half_fov_y_tan,
            self.alignment_bound_rad,
        )
    }

    /// Whether a normalized image mark lies within this geometry's field of view.
    fn within_fov(&self, x: f64, y: f64) -> bool {
        x.abs() <= self.half_fov_x_tan && y.abs() <= self.half_fov_y_tan
    }
}

/// The world's NED-down unit direction expressed in the camera optical frame:
/// rotate NED down into the body frame (the inverse aircraft attitude), then
/// body into camera (the extrinsics). All arithmetic is f64 — [`Quat::rotate`]
/// promotes — so the horizon coefficients do not lose precision to the f32
/// quaternion storage.
#[must_use]
pub fn down_in_camera(attitude_body_to_ned: Quat, body_to_camera: Quat) -> [f64; 3] {
    let down_body = attitude_body_to_ned.inverse().rotate([0.0, 0.0, 1.0]);
    body_to_camera.rotate(down_body)
}

/// Projects the horizon line for the given aircraft attitude and camera mount.
/// The coefficients are the NED-down direction in the camera frame.
#[must_use]
pub fn project_horizon(attitude_body_to_ned: Quat, geom: &ViewGeometry) -> HorizonLine {
    let [a, b, c] = down_in_camera(attitude_body_to_ned, geom.body_to_camera);
    HorizonLine { a, b, c }
}

/// Projects the flight-path marker at the ground-referenced NED velocity
/// direction. Returns `None` when the speed is below `MIN_SPEED_MPS` (no defined
/// direction) or the velocity direction is not in front of the camera (removed
/// rather than drawn behind the eye).
#[must_use]
pub fn project_flight_path(
    attitude_body_to_ned: Quat,
    velocity_ned_mps: [f64; 3],
    geom: &ViewGeometry,
) -> Option<ScreenMark> {
    let speed = norm3(velocity_ned_mps);
    if speed < MIN_SPEED_MPS {
        return None;
    }
    let dir_ned = [
        velocity_ned_mps[0] / speed,
        velocity_ned_mps[1] / speed,
        velocity_ned_mps[2] / speed,
    ];
    let dir_body = attitude_body_to_ned.inverse().rotate(dir_ned);
    let f = geom.body_to_camera.rotate(dir_body);
    if f[2] <= FORWARD_EPS {
        return None;
    }
    let x = f[0] / f[2];
    let y = f[1] / f[2];
    Some(ScreenMark {
        x,
        y,
        within_fov: geom.within_fov(x, y),
    })
}

/// Projects one runway/path cue point — a finite ground feature given as an
/// offset in the local NED frame relative to the ownship, meters — through the
/// camera and the projection view's near/far clip policy.
///
/// Returns `None` when the point is behind the camera or outside the near/far
/// clip range (removed rather than drawn); a point in front but outside the field
/// is `Some` with `within_fov = false` (the off-scale indication).
#[must_use]
pub fn project_path_cue(
    attitude_body_to_ned: Quat,
    offset_ned_m: [f64; 3],
    near_far: NearFarPolicy,
    geom: &ViewGeometry,
) -> Option<ScreenMark> {
    let off_body = attitude_body_to_ned.inverse().rotate(offset_ned_m);
    let c = geom.body_to_camera.rotate(off_body);
    let depth = c[2];
    if depth <= FORWARD_EPS || !near_far.contains_depth(depth) {
        return None;
    }
    let x = c[0] / depth;
    let y = c[1] / depth;
    Some(ScreenMark {
        x,
        y,
        within_fov: geom.within_fov(x, y),
    })
}

fn norm3(v: [f64; 3]) -> f64 {
    libm::sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2])
}
