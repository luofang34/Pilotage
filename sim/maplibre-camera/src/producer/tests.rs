#![allow(clippy::expect_used, clippy::panic)]

use pilotage_sim_video::wire::{BridgeCameraCommand, BridgeGimbalPose};

use super::{DEFAULT_TILT_RAD, gimbal_pointing};

#[test]
fn the_reported_gimbal_wins_then_the_command_then_ahead_and_down() {
    let reported = BridgeGimbalPose {
        pan_rad: 0.25,
        tilt_rad: -0.5,
        zoom_detent: 0,
    };
    let command = BridgeCameraCommand {
        mode: 2,
        pan_rad: 1.0,
        tilt_rad: -1.0,
        zoom_detent: 3,
    };
    let (pointing, detent) = gimbal_pointing(Some(reported), Some(command));
    assert_eq!(
        (pointing.pan_rad, pointing.tilt_rad, detent),
        (0.25, -0.5, 0)
    );
    let (pointing, detent) = gimbal_pointing(None, Some(command));
    assert_eq!(
        (pointing.pan_rad, pointing.tilt_rad, detent),
        (1.0, -1.0, 3)
    );
    let (pointing, detent) = gimbal_pointing(None, None);
    assert_eq!(
        (pointing.pan_rad, pointing.tilt_rad, detent),
        (0.0, DEFAULT_TILT_RAD, 0)
    );
}
