use super::GimbalYawFrame;
use crate::wire;

#[test]
fn every_yaw_frame_round_trips_through_the_wire() {
    for frame in [GimbalYawFrame::Vehicle, GimbalYawFrame::Earth] {
        assert_eq!(
            GimbalYawFrame::from_wire(frame.to_wire() as i32),
            Some(frame)
        );
    }
}

#[test]
fn an_unspecified_or_unknown_yaw_frame_is_no_frame() {
    assert_eq!(
        GimbalYawFrame::from_wire(wire::GimbalYawFrame::Unspecified as i32),
        None
    );
    assert_eq!(GimbalYawFrame::from_wire(7), None);
}

#[test]
fn no_yaw_frame_is_the_wire_default() {
    // A host that never fills the field must not claim a frame.
    for frame in [GimbalYawFrame::Vehicle, GimbalYawFrame::Earth] {
        assert_ne!(frame.to_wire() as i32, 0);
    }
}
