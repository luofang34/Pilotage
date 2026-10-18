//! SET_ATTITUDE_TARGET body-rate demands out, and the FC's ATTITUDE_TARGET
//! report in. The report carries the collective thrust the FC commands at
//! this moment, which in a hover is the hover thrust.

use super::FcMessage;
use super::encoding::encode_frame_v2;

/// SET_ATTITUDE_TARGET message id.
const SET_ATTITUDE_TARGET_ID: u32 = 82;
/// ATTITUDE_TARGET_TYPEMASK_ATTITUDE_IGNORE: fly the body rates, not an
/// attitude.
const IGNORE_ATTITUDE: u8 = 128;

/// One body-rate demand and the component selected to receive it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyRateTarget {
    /// Roll rate, rad/s (body FRD).
    pub roll_rps: f32,
    /// Pitch rate, rad/s (body FRD).
    pub pitch_rps: f32,
    /// Yaw rate, rad/s (body FRD).
    pub yaw_rps: f32,
    /// Normalized collective thrust, clamped to `[0, 1]`.
    pub thrust: f32,
    /// Selected MAVLink system id.
    pub system_id: u8,
    /// Selected MAVLink component id.
    pub component_id: u8,
}

/// Encodes one body-rate demand: rates plus thrust, attitude ignored (the
/// identity quaternion is sent in its place).
///
/// Wire order (39 bytes): time_boot_ms @0, q\[4\] @4..20, body roll/pitch/
/// yaw rate @20..32, thrust @32, target_system @36, target_component @37,
/// type_mask @38.
#[must_use]
pub fn encode_body_rate_setpoint(seq: u8, time_boot_ms: u32, target: BodyRateTarget) -> [u8; 51] {
    let mut payload = [0u8; 39];
    payload[0..4].copy_from_slice(&time_boot_ms.to_le_bytes());
    payload[4..8].copy_from_slice(&1.0_f32.to_le_bytes());
    payload[20..24].copy_from_slice(&target.roll_rps.to_le_bytes());
    payload[24..28].copy_from_slice(&target.pitch_rps.to_le_bytes());
    payload[28..32].copy_from_slice(&target.yaw_rps.to_le_bytes());
    payload[32..36].copy_from_slice(&target.thrust.clamp(0.0, 1.0).to_le_bytes());
    payload[36] = target.system_id;
    payload[37] = target.component_id;
    payload[38] = IGNORE_ATTITUDE;
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
