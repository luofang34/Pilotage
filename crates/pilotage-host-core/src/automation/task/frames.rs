//! The intent frames the principal sends: the neutral intent, and the
//! running digest of the frames' wire bytes.

use pilotage_protocol::{ControlIntent, ReferenceFrame, ScopedControlFrame, VelocityIntent};
use sha2::{Digest as _, Sha256};

/// Zero velocity: the frame that demonstrates neutral to the session.
pub(super) fn neutral_intent() -> ControlIntent {
    ControlIntent::Velocity(VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx: 0.0,
        vy: 0.0,
        vz: 0.0,
        yaw_rate: 0.0,
    })
}

/// The digest after `frame`: SHA-256 of the previous digest and the
/// frame's wire envelope bytes. Two runs give the same digest only when
/// they send the same frames in the same order.
pub(super) fn chain_digest(previous: [u8; 32], frame: &ScopedControlFrame) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(previous);
    hasher.update(pilotage_protocol::encode_control_frame_envelope(frame));
    hasher.finalize().into()
}
