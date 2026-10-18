//! SET_ATTITUDE_TARGET for direct flight: body rates or an absolute
//! attitude, each with collective thrust. Also the FC's ATTITUDE_TARGET
//! report in: it carries the collective thrust the FC commands at this
//! moment, which in a hover is the hover thrust.

use super::FcMessage;
use super::encoding::encode_frame_v2;

/// SET_ATTITUDE_TARGET message id.
pub const SET_ATTITUDE_TARGET_ID: u32 = 82;
/// ATTITUDE_TARGET_TYPEMASK_ATTITUDE_IGNORE.
const ATTITUDE_IGNORE: u8 = 128;
/// The three body-rate ignore bits.
const BODY_RATES_IGNORE: u8 = 0b0000_0111;

/// What the attitude loop is asked to follow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AttitudeTargetKind {
    /// Body angular rates (roll, pitch, yaw), rad/s; the attitude field is
    /// ignored.
    BodyRates([f32; 3]),
    /// An absolute body-FRD → NED attitude (w, x, y, z); the rate fields
    /// are ignored.
    Attitude([f32; 4]),
}

impl AttitudeTargetKind {
    /// The absolute attitude of roll, pitch, and yaw Euler angles (rad,
    /// ZYX order).
    #[must_use]
    pub fn from_euler(roll_rad: f32, pitch_rad: f32, yaw_rad: f32) -> Self {
        let (sr, cr) = (roll_rad * 0.5).sin_cos();
        let (sp, cp) = (pitch_rad * 0.5).sin_cos();
        let (sy, cy) = (yaw_rad * 0.5).sin_cos();
        Self::Attitude([
            cr * cp * cy + sr * sp * sy,
            sr * cp * cy - cr * sp * sy,
            cr * sp * cy + sr * cp * sy,
            cr * cp * sy - sr * sp * cy,
        ])
    }
}

/// Serializes a SET_ATTITUDE_TARGET with collective `thrust` in `[0, 1]`
/// (clamped; NaN becomes zero).
///
/// Wire order (39 bytes): time_boot_ms u32 @0, q\[4\] @4..20, body
/// roll/pitch/yaw rate @20..32, thrust @32, target_system @36,
/// target_component @37, type_mask @38.
#[must_use]
pub fn encode_attitude_target(
    seq: u8,
    time_boot_ms: u32,
    kind: AttitudeTargetKind,
    thrust: f32,
    target_system: u8,
    target_component: u8,
) -> [u8; 51] {
    let (quat, rates, type_mask) = match kind {
        AttitudeTargetKind::BodyRates(rates) => ([1.0, 0.0, 0.0, 0.0], rates, ATTITUDE_IGNORE),
        AttitudeTargetKind::Attitude(quat) => (quat, [0.0; 3], BODY_RATES_IGNORE),
    };
    let mut payload = [0u8; 39];
    payload[0..4].copy_from_slice(&time_boot_ms.to_le_bytes());
    for (index, value) in quat.iter().chain(rates.iter()).enumerate() {
        let at = 4 + index * 4;
        payload[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    let thrust = if thrust.is_nan() {
        0.0
    } else {
        thrust.clamp(0.0, 1.0)
    };
    payload[32..36].copy_from_slice(&thrust.to_le_bytes());
    payload[36] = target_system;
    payload[37] = target_component;
    payload[38] = type_mask;
    let mut frame = [0u8; 51];
    encode_frame_v2(seq, SET_ATTITUDE_TARGET_ID, &payload, 49, &mut frame);
    frame
}

/// ATTITUDE_TARGET wire order (37 bytes): time_boot_ms @0, q\[4\] @4..20,
/// body rates @20..32, thrust @32, type_mask @36.
pub(super) fn decode_attitude_target(payload: &[u8]) -> FcMessage {
    let word = |at: usize| {
        let mut bytes = [0u8; 4];
        for (index, slot) in bytes.iter_mut().enumerate() {
            *slot = payload.get(at + index).copied().unwrap_or(0);
        }
        bytes
    };
    FcMessage::AttitudeTarget {
        time_boot_ms: u32::from_le_bytes(word(0)),
        thrust: f32::from_le_bytes(word(32)),
    }
}
