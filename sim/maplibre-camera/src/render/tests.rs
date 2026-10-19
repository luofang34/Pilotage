#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use cgmath::{Quaternion, Rad, Rotation3};
use maplibre::style::Style;

use super::Camera;
use crate::geometry::{BodyPose, Height, Pointing};
use crate::rig::CameraSpec;

const GROUND: [u8; 3] = [255, 128, 0];

/// Ground of one colour under an undrawn sky, and nothing to fetch. A
/// style sky would colour the exposed background too, where no terrain
/// mesh covers it.
fn style() -> Style {
    serde_json::from_value(serde_json::json!({
        "version": 8,
        "projection": {"type": "mercator"},
        "sources": {},
        "layers": [{"id": "ground", "type": "background", "paint": {"background-color": "#ff8000"}}],
    }))
    .expect("style")
}

fn camera(runtime: &tokio::runtime::Runtime) -> Camera {
    let cache = std::env::temp_dir().join("maplibre-camera-render-test");
    let loader = maplibre::io::resource_loader::SharedLoader::new(
        maplibre::platform::http_client::ReqwestHttpClient::new(Some(cache)),
    );
    runtime
        .block_on(Camera::new(CameraSpec::gimbal((160, 120)), style(), loader))
        .expect("camera")
}

fn pose() -> BodyPose {
    BodyPose {
        latitude_deg: 40.577,
        longitude_deg: -74.738,
        height: Height::Msl(300.0),
        attitude: Quaternion::from_angle_z(Rad(0.4)),
    }
}

/// Share of pixels that are the ground colour, in the top and the bottom
/// halves of the image.
fn ground_share(rgb: &[u8]) -> (f64, f64) {
    let half = rgb.len() / 2;
    let share = |part: &[u8]| {
        let pixels = part.as_chunks::<3>().0;
        let ground = pixels.iter().filter(|pixel| **pixel == GROUND).count();
        ground as f64 / pixels.len() as f64
    };
    (share(&rgb[..half]), share(&rgb[half..]))
}

#[test]
fn a_camera_sees_ground_below_the_horizon_and_sky_above_it() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let _entered = runtime.enter();
    let mut camera = camera(&runtime);
    let mut frame = 0_u64;
    let mut view = |camera: &mut Camera, tilt_rad: f64| {
        frame = frame.wrapping_add(1);
        let pointing = Pointing {
            pan_rad: 0.0,
            tilt_rad,
        };
        let rgb = camera
            .render_blocking(&pose(), pointing, 0, Duration::from_millis(16 * frame))
            .expect("frame");
        assert_eq!(rgb.len(), 160 * 120 * 3);
        ground_share(rgb)
    };

    let (top, bottom) = view(&mut camera, -std::f64::consts::FRAC_PI_2);
    assert!(
        top > 0.99 && bottom > 0.99,
        "straight down is all ground: {top} {bottom}"
    );

    let (top, bottom) = view(&mut camera, 0.0);
    assert!(top < 0.05, "above a level horizon is sky: {top}");
    assert!(bottom > 0.95, "below it is ground: {bottom}");

    let (top, bottom) = view(&mut camera, 0.6);
    assert!(
        top < 0.01 && bottom < 0.01,
        "looking up sees no ground: {top} {bottom}"
    );
}

#[test]
fn a_height_above_unknown_ground_keeps_drawing_but_sends_nothing() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let _entered = runtime.enter();
    let mut camera = camera(&runtime);
    let above_ground = BodyPose {
        height: Height::Agl(60.0),
        ..pose()
    };
    // The offline style has no terrain, so the ground never becomes known;
    // every frame still draws and reports it rather than wedging.
    for frame in 1..=3_u64 {
        let result = camera.render_blocking(
            &above_ground,
            Pointing::default(),
            0,
            Duration::from_millis(16 * frame),
        );
        assert!(
            matches!(result, Err(super::FrameError::GroundUnknown)),
            "frame {frame}: {:?}",
            result.map(<[u8]>::len)
        );
    }
}

#[test]
fn a_lens_below_the_map_ground_is_raised_onto_it() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let _entered = runtime.enter();
    let mut camera = camera(&runtime);
    // A lens 0.1 m above a body origin 0.2 m under the ground stands 0.5 m
    // above the ground.
    let raised = camera.above_ground(99.8, 0.1, Some(100.0));
    assert!((raised + 0.1 - 100.5).abs() < 1e-9, "{raised}");
    // Clear of the ground, or with no ground known, nothing moves.
    assert_eq!(camera.above_ground(120.0, 0.1, Some(100.0)), 120.0);
    assert_eq!(camera.above_ground(50.0, 0.1, None), 50.0);
}
