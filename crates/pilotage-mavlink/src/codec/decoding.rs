//! Payload decoding with MAVLink 2 trailing-zero extension.

use super::{
    ATTITUDE_QUATERNION_ID, AVIATE_ESTIMATOR_STATUS_ID, COMMAND_ACK_ID, ESTIMATOR_STATUS_ID,
    FcMessage, GIMBAL_DEVICE_ATTITUDE_STATUS_ID, GNSS_RAW_ID, HEARTBEAT_ID,
    HIL_STATE_QUATERNION_ID, LOCAL_POSITION_NED_ID, SCALED_PRESSURE_ID,
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

/// HIL_STATE_QUATERNION wire order (64-bit first, then arrays,
/// then 32-bit, then 16-bit): time_usec @0, q[4] @8..24,
/// roll/pitch/yawspeed @24..36, lat/lon/alt @36..48,
/// vx/vy/vz i16 cm/s @48..54, then acceleration fields.
fn decode_sim_truth(payload: &[u8]) -> FcMessage {
    FcMessage::SimTruth {
        time_usec: u64_at(payload, 0),
        quat_wxyz: [
            f32_at(payload, 8),
            f32_at(payload, 12),
            f32_at(payload, 16),
            f32_at(payload, 20),
        ],
        vel_ned_mps: [
            f32::from(u16_at(payload, 48) as i16) / 100.0,
            f32::from(u16_at(payload, 50) as i16) / 100.0,
            f32::from(u16_at(payload, 52) as i16) / 100.0,
        ],
        lat_lon_alt: [
            u32_at(payload, 36) as i32,
            u32_at(payload, 40) as i32,
            u32_at(payload, 44) as i32,
        ],
    }
}

/// Wire order: time_boot_ms u32 @0, q[4] @4..20, angular
/// velocity x/y/z @20..32, failure_flags u32 @32, flags u16
/// @36, targets @38..40, then v2 extension fields this decoder
/// ignores (zero-truncated payloads shorter than 40 still
/// decode via the zero-extending accessors).
fn decode_gimbal_status(payload: &[u8]) -> FcMessage {
    FcMessage::GimbalDeviceAttitudeStatus {
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
        failure_flags: u32_at(payload, 32),
        flags: u16_at(payload, 36),
    }
}

/// GPS_RAW_INT wire order: time_usec u64 @0, lat/lon in degrees*1e7 @8/@12,
/// sea-level altitude @16, accuracy and course fields, then fix_type u8 @28
/// and satellite count @29, and the version-2 extension fields this lane
/// reads: alt_ellipsoid i32 @30 and the 1-sigma horizontal and vertical
/// accuracies @34 and @38.
///
/// The length gate reaches the fix type and no further. MAVLink 2 trims
/// trailing zero BYTES, which can cut into the middle of a value whose high
/// bytes are zero, so no offset past the first zero byte can be required to
/// be present. What a sender means by a zero is settled per field instead:
/// an accuracy of zero is unstated rather than perfect, and a position of
/// zero is refused where the fix is built.
fn decode_gnss_fix(payload: &[u8]) -> Option<FcMessage> {
    /// Below a three-dimensional fix the receiver has no height and may
    /// have no position. It says so here and nowhere else.
    const FIX_TYPE_3D: u8 = 3;

    if payload.len() <= 28 || payload[28] < FIX_TYPE_3D {
        return None;
    }
    Some(FcMessage::GnssFix {
        time_usec: u64_at(payload, 0),
        lat_lon: [u32_at(payload, 8) as i32, u32_at(payload, 12) as i32],
        alt_ellipsoid_mm: u32_at(payload, 30) as i32,
        accuracy_mm: [u32_at(payload, 34), u32_at(payload, 38)],
    })
}

pub(super) fn decode_known(msg_id: u32, payload: &[u8]) -> Option<FcMessage> {
    match msg_id {
        HEARTBEAT_ID => Some(FcMessage::Heartbeat {
            // Payload: custom_mode u32 @0, type @4, autopilot @5,
            // base_mode @6 (bit 0x80 = SAFETY_ARMED).
            armed: payload.get(6).is_some_and(|b| b & 0x80 != 0),
        }),
        COMMAND_ACK_ID => Some(FcMessage::CommandAck {
            command: u16::from(payload.first().copied().unwrap_or(0))
                | (u16::from(payload.get(1).copied().unwrap_or(0)) << 8),
            result: payload.get(2).copied().unwrap_or(0),
            // v2 extension fields: zero when the sender truncated them,
            // which consumers treat as "unaddressed".
            target_system: payload.get(8).copied().unwrap_or(0),
            target_component: payload.get(9).copied().unwrap_or(0),
        }),
        ATTITUDE_QUATERNION_ID => Some(FcMessage::AttitudeQuaternion {
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
        SCALED_PRESSURE_ID => Some(FcMessage::ScaledPressure {
            time_boot_ms: u32_at(payload, 0),
            press_abs_hpa: f32_at(payload, 4),
            temperature_cdeg: u16_at(payload, 12) as i16,
        }),
        HIL_STATE_QUATERNION_ID => Some(decode_sim_truth(payload)),
        GNSS_RAW_ID => decode_gnss_fix(payload),
        LOCAL_POSITION_NED_ID => Some(FcMessage::LocalPositionNed {
            time_boot_ms: u32_at(payload, 0),
            pos_ned_m: [f32_at(payload, 4), f32_at(payload, 8), f32_at(payload, 12)],
            vel_ned_mps: [
                f32_at(payload, 16),
                f32_at(payload, 20),
                f32_at(payload, 24),
            ],
        }),
        ESTIMATOR_STATUS_ID => Some(FcMessage::EstimatorStatus {
            time_usec: u64_at(payload, 0),
            flags: u16_at(payload, 40),
        }),
        AVIATE_ESTIMATOR_STATUS_ID => Some(FcMessage::AviateEstimatorStatus {
            time_usec: u64_at(payload, 0),
            valid_flags: payload.get(8).copied().unwrap_or(0),
            quality: payload.get(9).copied().unwrap_or(0),
        }),
        GIMBAL_DEVICE_ATTITUDE_STATUS_ID => Some(decode_gimbal_status(payload)),
        _ => None,
    }
}
