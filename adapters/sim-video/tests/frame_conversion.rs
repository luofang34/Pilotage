//! A producer frame's calibration id survives conversion, and zero
//! states none.
#![allow(clippy::expect_used, clippy::panic)]

use pilotage_adapter_api::{CalibrationId, UnstampedFrame};
use pilotage_sim_video::wire::BridgeFrame;

fn frame(calibration_id: u32) -> BridgeFrame {
    BridgeFrame {
        width: 1,
        height: 1,
        pixel_format: "RGB_INT8".to_owned(),
        sim_time_ns: 5,
        rgb: vec![1, 2, 3],
        camera_id: 2,
        calibration_id,
    }
}

#[test]
fn a_frame_carries_the_calibration_its_producer_captured_it_with() {
    let converted = UnstampedFrame::from(frame(0x5A43_0002));
    assert_eq!(converted.calibration_id, Some(CalibrationId(0x5A43_0002)));
    assert_eq!((converted.camera_id, converted.time_ns), (2, 5));
}

#[test]
fn a_zero_calibration_id_states_none() {
    assert_eq!(UnstampedFrame::from(frame(0)).calibration_id, None);
}
