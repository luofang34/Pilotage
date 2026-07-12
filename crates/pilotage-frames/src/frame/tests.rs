#![allow(clippy::expect_used, clippy::panic)]

use super::FrameId;
use crate::error::FrameError;

const ALL: [FrameId; 8] = [
    FrameId::Body,
    FrameId::Installation,
    FrameId::Ned,
    FrameId::Ecef,
    FrameId::Eci,
    FrameId::Lvlh,
    FrameId::Rtn,
    FrameId::TargetRelative,
];

#[test]
fn every_frame_round_trips_the_wire() {
    for frame in ALL {
        assert_eq!(FrameId::from_u8(frame.to_u8()), Ok(frame));
    }
}

#[test]
fn unknown_frame_codes_fail_closed() {
    for code in 8..=255u8 {
        assert_eq!(
            FrameId::from_u8(code),
            Err(FrameError::UnknownFrame { code }),
            "code {code} must not decode"
        );
    }
}
