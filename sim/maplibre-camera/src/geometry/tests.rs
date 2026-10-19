#![allow(clippy::expect_used, clippy::panic)]

use std::f64::consts::FRAC_PI_2;

use cgmath::{InnerSpace, Matrix4, Quaternion, Rotation3, Vector3, Vector4};
use pilotage_sim_video::wire::BridgeViewPose;

use super::{BodyPose, Height, Pointing, PoseError, world_from_eye};
use crate::rig::{CameraSpec, Mount};

/// The NED rotation of a heading, pitch, and roll (Z-Y-X).
fn attitude(heading: f64, pitch: f64, roll: f64) -> Quaternion<f64> {
    Quaternion::from_angle_z(cgmath::Rad(heading))
        * Quaternion::from_angle_y(cgmath::Rad(pitch))
        * Quaternion::from_angle_x(cgmath::Rad(roll))
}

fn pose(attitude: Quaternion<f64>) -> BodyPose {
    BodyPose {
        latitude_deg: 40.5,
        longitude_deg: -74.7,
        height: Height::Msl(100.0),
        attitude,
    }
}

/// Where an eye looks and which way is up for it, in east-north-up.
fn look(matrix: Matrix4<f64>) -> (Vector3<f64>, Vector3<f64>) {
    let forward = (matrix * Vector4::new(0.0, 0.0, -1.0, 0.0)).truncate();
    let up = (matrix * Vector4::new(0.0, 1.0, 0.0, 0.0)).truncate();
    (forward, up)
}

fn close(a: Vector3<f64>, b: Vector3<f64>) {
    assert!((a - b).magnitude() < 1e-9, "{a:?} vs {b:?}");
}

fn level_camera() -> CameraSpec {
    CameraSpec {
        mount: Mount::Body { pitch_rad: 0.0 },
        ..CameraSpec::fpv((640, 480))
    }
}

#[test]
fn a_level_forward_camera_looks_where_the_body_heads() {
    let camera = level_camera();
    let (forward, up) = look(world_from_eye(
        &camera,
        &pose(attitude(0.0, 0.0, 0.0)),
        Pointing::default(),
    ));
    close(forward, Vector3::new(0.0, 1.0, 0.0));
    close(up, Vector3::unit_z());
    // Heading east, the view turns east; nose up 30 degrees, it rises.
    let (forward, _) = look(world_from_eye(
        &camera,
        &pose(attitude(FRAC_PI_2, 0.0, 0.0)),
        Pointing::default(),
    ));
    close(forward, Vector3::unit_x());
    let (forward, _) = look(world_from_eye(
        &camera,
        &pose(attitude(0.0, 0.5, 0.0)),
        Pointing::default(),
    ));
    close(forward, Vector3::new(0.0, 0.5_f64.cos(), 0.5_f64.sin()));
    // A right roll leans the camera's up toward the right (east).
    let (_, up) = look(world_from_eye(
        &camera,
        &pose(attitude(0.0, 0.0, 0.3)),
        Pointing::default(),
    ));
    close(up, Vector3::new(0.3_f64.sin(), 0.0, 0.3_f64.cos()));
}

#[test]
fn the_mount_pitch_and_lens_offset_ride_with_the_body() {
    let camera = CameraSpec::fpv((640, 480));
    let matrix = world_from_eye(
        &camera,
        &pose(attitude(FRAC_PI_2, 0.0, 0.0)),
        Pointing::default(),
    );
    let (forward, _) = look(matrix);
    let down = 10_f64.to_radians();
    close(forward, Vector3::new(down.cos(), 0.0, -down.sin()));
    // Lens 0.15 m forward and 0.05 m below the body origin, heading east.
    close(matrix.w.truncate(), Vector3::new(0.15, 0.0, -0.05));
}

#[test]
fn a_gimbal_holds_the_horizon_and_pans_from_the_heading() {
    let camera = CameraSpec::gimbal((640, 480));
    let rolled = pose(attitude(FRAC_PI_2, 0.2, 0.4));
    let ahead = Pointing {
        pan_rad: 0.0,
        tilt_rad: 0.0,
    };
    let (forward, up) = look(world_from_eye(&camera, &rolled, ahead));
    close(forward, Vector3::unit_x());
    close(up, Vector3::unit_z());
    let right_and_down = Pointing {
        pan_rad: FRAC_PI_2,
        tilt_rad: -FRAC_PI_2 / 2.0,
    };
    let (forward, _) = look(world_from_eye(&camera, &rolled, right_and_down));
    let half = (0.5_f64).sqrt();
    close(forward, Vector3::new(0.0, -half, -half));
}

fn wire_pose(vertical_datum: u32) -> BridgeViewPose {
    BridgeViewPose {
        sim_time_ns: 1,
        latitude_deg: 40.5,
        longitude_deg: -74.7,
        altitude_m: 80.0,
        vertical_datum,
        q_w: 2.0_f64.sqrt() / 2.0,
        q_x: 0.0,
        q_y: 0.0,
        q_z: 2.0_f64.sqrt() / 2.0,
        gimbal: None,
    }
}

#[test]
fn only_heights_the_terrain_can_place_are_accepted() {
    let msl = BodyPose::try_from(&wire_pose(2)).expect("MSL");
    assert_eq!(msl.height, Height::Msl(80.0));
    let agl = BodyPose::try_from(&wire_pose(3)).expect("AGL");
    assert_eq!(agl.height, Height::Agl(80.0));
    for datum in [0, 1, 4, 5, 6, 99] {
        assert_eq!(
            BodyPose::try_from(&wire_pose(datum)),
            Err(PoseError::Datum(datum))
        );
    }
    let mut bad = wire_pose(2);
    bad.q_w = 0.0;
    assert_eq!(BodyPose::try_from(&bad), Err(PoseError::NotFinite));
    let mut bad = wire_pose(2);
    bad.latitude_deg = f64::NAN;
    assert_eq!(BodyPose::try_from(&bad), Err(PoseError::NotFinite));
}
