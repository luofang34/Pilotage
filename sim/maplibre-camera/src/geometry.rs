//! From the vehicle's true pose to each camera's eye in the map's scene.
//!
//! The map's scene frame measures metres east, north, and up from an
//! anchor; its eyes follow the OpenGL convention: x right, y up, looking
//! along negative z. The vehicle's attitude rotates body FRD into local
//! NED. The scene is anchored at the body origin, so an eye's translation
//! is only its lens offset.

use cgmath::{Matrix3, Matrix4, Quaternion, Vector3};
use pilotage_sim_video::wire::BridgeViewPose;

use crate::rig::{CameraSpec, Mount};

/// `pilotage.v1` vertical-datum code for a height above mean sea level.
const DATUM_MSL: u32 = 2;
/// `pilotage.v1` vertical-datum code for a height above the ground.
const DATUM_AGL: u32 = 3;

/// The reference of a pose's height, as the map can place it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Height {
    /// Metres above mean sea level, the reference of the Terrarium terrain.
    Msl(f64),
    /// Metres above the terrain under the vehicle.
    Agl(f64),
}

/// A pose the producer can render from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyPose {
    /// Latitude, degrees.
    pub latitude_deg: f64,
    /// Longitude, degrees.
    pub longitude_deg: f64,
    /// Height of the body origin.
    pub height: Height,
    /// Body FRD to NED rotation.
    pub attitude: Quaternion<f64>,
}

/// Why a pose cannot be rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PoseError {
    /// A vertical datum the terrain cannot place: an ellipsoid height needs a
    /// geoid model, and a local height an origin, neither of which the pose
    /// names.
    #[error("vertical datum {0} cannot be placed against the terrain")]
    Datum(u32),
    /// A non-finite position or a quaternion far from unit length.
    #[error("the pose is not a finite position and attitude")]
    NotFinite,
}

impl TryFrom<&BridgeViewPose> for BodyPose {
    type Error = PoseError;

    fn try_from(pose: &BridgeViewPose) -> Result<Self, PoseError> {
        let height = match pose.vertical_datum {
            DATUM_MSL => Height::Msl(pose.altitude_m),
            DATUM_AGL => Height::Agl(pose.altitude_m),
            other => return Err(PoseError::Datum(other)),
        };
        let attitude = Quaternion::new(pose.q_w, pose.q_x, pose.q_y, pose.q_z);
        let norm = attitude
            .s
            .hypot(attitude.v.x)
            .hypot(attitude.v.y)
            .hypot(attitude.v.z);
        let finite = [pose.latitude_deg, pose.longitude_deg, pose.altitude_m]
            .iter()
            .all(|value| value.is_finite());
        if !finite || !norm.is_finite() || (norm - 1.0).abs() > 0.01 {
            return Err(PoseError::NotFinite);
        }
        Ok(Self {
            latitude_deg: pose.latitude_deg,
            longitude_deg: pose.longitude_deg,
            height,
            attitude: attitude / norm,
        })
    }
}

/// Where a gimbal camera points.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pointing {
    /// Pan, radians, positive right of the body heading.
    pub pan_rad: f64,
    /// Tilt, radians, positive up from the horizon.
    pub tilt_rad: f64,
}

/// Local NED to the scene's east-north-up axes.
fn enu_from_ned() -> Matrix3<f64> {
    Matrix3::new(0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, -1.0)
}

/// An OpenGL eye to the FRD axes of the camera it looks through: x right
/// is FRD right, y up is FRD up, and the eye looks along FRD forward.
fn frd_from_eye() -> Matrix3<f64> {
    Matrix3::new(0.0, 1.0, 0.0, 0.0, 0.0, -1.0, -1.0, 0.0, 0.0)
}

/// Rotation about FRD down (z) by `angle`, positive right.
fn about_down(angle: f64) -> Matrix3<f64> {
    let (sin, cos) = angle.sin_cos();
    Matrix3::new(cos, sin, 0.0, -sin, cos, 0.0, 0.0, 0.0, 1.0)
}

/// Rotation about FRD right (y) by `angle`, positive nose up.
fn about_right(angle: f64) -> Matrix3<f64> {
    let (sin, cos) = angle.sin_cos();
    Matrix3::new(cos, 0.0, -sin, 0.0, 1.0, 0.0, sin, 0.0, cos)
}

/// Heading of a body FRD to NED attitude, radians from north toward east.
fn heading([w, x, y, z]: [f64; 4]) -> f64 {
    (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z))
}

/// The camera's pose in the scene: eye space to east-north-up metres from
/// the body origin.
#[must_use]
pub fn world_from_eye(camera: &CameraSpec, pose: &BodyPose, pointing: Pointing) -> Matrix4<f64> {
    let ned_from_body = Matrix3::from(pose.attitude);
    let ned_from_camera = match camera.mount {
        Mount::Body { pitch_rad } => ned_from_body * about_right(pitch_rad),
        Mount::Gimbal => {
            let q = pose.attitude;
            let yaw = heading([q.s, q.v.x, q.v.y, q.v.z]);
            about_down(yaw + pointing.pan_rad) * about_right(pointing.tilt_rad)
        }
    };
    let rotation = enu_from_ned() * ned_from_camera * frd_from_eye();
    let lens = enu_from_ned() * ned_from_body * Vector3::from(camera.mount_body_m);
    let mut matrix = Matrix4::from(rotation);
    matrix.w = lens.extend(1.0);
    matrix
}

#[cfg(test)]
mod tests;
