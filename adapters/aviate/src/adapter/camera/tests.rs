//! Which camera producer a session's `PILOTAGE_AVIATE_CAMERA` names.

use super::{CameraMode, mode_from};

/// Only a launcher that asks for Gazebo gets the gz sidecar: a session
/// that names no camera runs none.
#[test]
fn the_gz_sidecar_runs_only_when_named() {
    assert_eq!(mode_from(None), CameraMode::Off);
    assert_eq!(mode_from(Some("off")), CameraMode::Off);
    assert_eq!(mode_from(Some("gazebo")), CameraMode::Gazebo);
    assert_eq!(mode_from(Some("xplane-plugin")), CameraMode::XPlanePlugin);
    for unknown in ["on", "maplibre", ""] {
        assert_eq!(mode_from(Some(unknown)), CameraMode::Off, "{unknown:?}");
    }
}
