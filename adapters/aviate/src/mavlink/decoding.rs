//! Payload decoding with MAVLink 2 trailing-zero extension.

use super::{
    ATTITUDE_QUATERNION_ID, AVIATE_ESTIMATOR_STATUS_ID, AviateMessage, COMMAND_ACK_ID,
    ESTIMATOR_STATUS_ID, HEARTBEAT_ID, LOCAL_POSITION_NED_ID,
};

fn f32_at(payload: &[u8], off: usize) -> f32 {
    let mut bytes = [0_u8; 4];
    for (index, slot) in bytes.iter_mut().enumerate() {
        *slot = payload.get(off + index).copied().unwrap_or(0);
    }
    f32::from_le_bytes(bytes)
}

fn u32_at(payload: &[u8], off: usize) -> u32 {
    let mut bytes = [0_u8; 4];
    for (index, slot) in bytes.iter_mut().enumerate() {
        *slot = payload.get(off + index).copied().unwrap_or(0);
    }
    u32::from_le_bytes(bytes)
}

fn u64_at(payload: &[u8], off: usize) -> u64 {
    let mut bytes = [0_u8; 8];
    for (index, slot) in bytes.iter_mut().enumerate() {
        *slot = payload.get(off + index).copied().unwrap_or(0);
    }
    u64::from_le_bytes(bytes)
}

fn u16_at(payload: &[u8], off: usize) -> u16 {
    let mut bytes = [0_u8; 2];
    for (index, slot) in bytes.iter_mut().enumerate() {
        *slot = payload.get(off + index).copied().unwrap_or(0);
    }
    u16::from_le_bytes(bytes)
}

pub(super) fn decode_known(msg_id: u32, payload: &[u8]) -> Option<AviateMessage> {
    match msg_id {
        HEARTBEAT_ID => Some(AviateMessage::Heartbeat {
            // Payload: custom_mode u32 @0, type @4, autopilot @5,
            // base_mode @6 (bit 0x80 = SAFETY_ARMED).
            armed: payload.get(6).is_some_and(|b| b & 0x80 != 0),
        }),
        COMMAND_ACK_ID => Some(AviateMessage::CommandAck {
            command: u16::from(payload.first().copied().unwrap_or(0))
                | (u16::from(payload.get(1).copied().unwrap_or(0)) << 8),
            result: payload.get(2).copied().unwrap_or(0),
        }),
        ATTITUDE_QUATERNION_ID => Some(AviateMessage::AttitudeQuaternion {
            time_boot_ms: u32_at(payload, 0),
            quat_wxyz: [
                f32_at(payload, 4),
                f32_at(payload, 8),
                f32_at(payload, 12),
                f32_at(payload, 16),
            ],
            rates_rps: [
                f32_at(payload, 20),
                f32_at(payload, 24),
                f32_at(payload, 28),
            ],
        }),
        LOCAL_POSITION_NED_ID => Some(AviateMessage::LocalPositionNed {
            time_boot_ms: u32_at(payload, 0),
            pos_ned_m: [f32_at(payload, 4), f32_at(payload, 8), f32_at(payload, 12)],
            vel_ned_mps: [
                f32_at(payload, 16),
                f32_at(payload, 20),
                f32_at(payload, 24),
            ],
        }),
        ESTIMATOR_STATUS_ID => Some(AviateMessage::EstimatorStatus {
            time_usec: u64_at(payload, 0),
            flags: u16_at(payload, 40),
        }),
        AVIATE_ESTIMATOR_STATUS_ID => Some(AviateMessage::AviateEstimatorStatus {
            time_usec: u64_at(payload, 0),
            valid_flags: payload.get(8).copied().unwrap_or(0),
            quality: payload.get(9).copied().unwrap_or(0),
        }),
        _ => None,
    }
}
