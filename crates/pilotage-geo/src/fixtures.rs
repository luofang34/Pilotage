//! Explicit identities shared by geometry fixtures.
#![cfg(test)]
use indicate_frames::{ClockDomain, ClockId, FrameId, FrameKind};

pub(crate) fn clock(domain: ClockDomain) -> ClockId {
    ClockId::new(domain, u128::from(domain as u8) + 1).expect("fixture clock")
}

pub(crate) fn frame(kind: FrameKind) -> FrameId {
    FrameId::new(kind, u128::from(kind.to_u8()) + 1).expect("fixture frame")
}
