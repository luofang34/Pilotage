#![allow(clippy::expect_used, clippy::panic)]

use base64::Engine;
use pilotage_camera_calibration::{sim_fpv_calibration, to_canonical};

use super::{artifact_json, calibration, write_artifacts};
use crate::rig::{CameraSpec, Mount};

#[test]
fn every_detent_calibration_validates_and_hashes_its_own_bytes() {
    let gimbal = CameraSpec::gimbal((640, 480));
    for detent in 0..gimbal.detents_hfov_rad.len() {
        let calibration = calibration(&gimbal, detent).expect("valid");
        let lens = gimbal.intrinsics(detent);
        assert_eq!(calibration.geometry.intrinsics.focal_x_px, lens.focal_px);
        let fov = calibration.geometry.field_of_view();
        assert!((fov.horizontal_rad - gimbal.detents_hfov_rad[detent]).abs() < 1e-9);
        assert_eq!(
            calibration.identity.calibration_id.0,
            gimbal.calibration_id(detent)
        );
        assert_eq!(calibration.identity.camera_id, gimbal.camera_id);
        calibration
            .verify(calibration.content_hash(), 1_800_000_000_000_000_000)
            .expect("hash and window verify");
    }
}

#[test]
fn a_level_forward_mount_stores_the_simulator_forward_rotation() {
    let level = CameraSpec {
        mount: Mount::Body { pitch_rad: 0.0 },
        ..CameraSpec::fpv((640, 480))
    };
    let stored = calibration(&level, 0)
        .expect("valid")
        .geometry
        .extrinsics
        .rotation_quat_wxyz;
    let published = sim_fpv_calibration().geometry.extrinsics.rotation_quat_wxyz;
    for (stored, published) in stored.iter().zip(published) {
        assert!(
            (stored - published).abs() < 1e-12,
            "{stored:?} vs {published:?}"
        );
    }
}

#[test]
fn an_artifact_carries_the_canonical_bytes_and_their_hash() {
    let fpv = CameraSpec::fpv((640, 480));
    let calibration = calibration(&fpv, 0).expect("valid");
    let document: serde_json::Value =
        serde_json::from_str(&artifact_json(&calibration)).expect("JSON");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(document["canonicalBase64"].as_str().expect("bytes"))
        .expect("base64");
    assert_eq!(bytes, to_canonical(&calibration));
    let hash: String = calibration
        .content_hash()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(document["recordedHashHex"], hash);
}

#[test]
fn artifacts_are_written_one_per_detent() {
    let dir = std::env::temp_dir().join(format!("maplibre-camera-cal-{}", std::process::id()));
    let cameras = [CameraSpec::fpv((640, 480)), CameraSpec::gimbal((640, 480))];
    let written = write_artifacts(&dir, &cameras).expect("written");
    assert_eq!(written.len(), 5);
    assert!(written.iter().all(|path| path.exists()));
    std::fs::remove_dir_all(&dir).expect("clean up");
}
