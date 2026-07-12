#![allow(clippy::expect_used, clippy::panic)]

use super::ned_attitude;
use crate::error::FrameError;
use crate::frame::FrameId;
use crate::rotation::Quat;
use crate::tagged::{Attitude, Tagged};
use crate::time::{ClockDomain, Epoch, TimeScale};

fn tagged(frame: FrameId, value: Quat) -> Attitude<u32> {
    Tagged {
        frame,
        epoch: Epoch {
            clock: ClockDomain::Simulation,
            scale: TimeScale::Monotonic,
            nanos: 1,
        },
        meta: 42,
        value,
    }
}

#[test]
fn ned_reference_passes_through_bit_identical() {
    let q = Quat {
        w: 0.5,
        x: 0.5,
        y: 0.5,
        z: 0.5,
    };
    let out = ned_attitude(&tagged(FrameId::Ned, q)).expect("ned accepted");
    assert_eq!(out, q, "the adapter hands the quaternion through unchanged");
}

#[test]
fn non_ned_references_are_refused_not_relabeled() {
    for frame in [
        FrameId::Eci,
        FrameId::Ecef,
        FrameId::Lvlh,
        FrameId::Rtn,
        FrameId::TargetRelative,
        FrameId::Body,
        FrameId::Installation,
    ] {
        assert_eq!(
            ned_attitude(&tagged(frame, Quat::IDENTITY)),
            Err(FrameError::FrameMismatch {
                expected: FrameId::Ned,
                found: frame,
            }),
            "{frame:?} has no implicit horizon"
        );
    }
}
