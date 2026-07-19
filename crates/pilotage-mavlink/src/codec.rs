//! Minimal MAVLink 2.0 frame math for Aviate's telemetry subset
//! (ADR-0018): pure byte functions, no I/O, no allocation on the parse
//! path beyond the caller's message vector.
//!
//! The decoded telemetry set includes liveness, estimator values, and both
//! standard and Aviate-private estimator status. Anything else is counted
//! and skipped. CRC is the MAVLink X.25 accumulator seeded with
//! each message's CRC_EXTRA (values cross-checked against Aviate's
//! `aviate-link` implementation, the peer this parser must interoperate
//! with byte-for-byte).

mod decoding;
use decoding::decode_known;

/// MAVLink 2.0 start-of-frame marker.
pub const MAGIC_V2: u8 = 0xFD;
/// HEARTBEAT message id.
pub const HEARTBEAT_ID: u32 = 0;
/// ATTITUDE_QUATERNION message id.
pub const ATTITUDE_QUATERNION_ID: u32 = 31;
/// LOCAL_POSITION_NED message id.
pub const LOCAL_POSITION_NED_ID: u32 = 32;
/// COMMAND_ACK message id (arm/disarm feedback).
pub const COMMAND_ACK_ID: u32 = 77;
/// Standard ESTIMATOR_STATUS message id.
pub const ESTIMATOR_STATUS_ID: u32 = 230;
/// Aviate's lossless estimator authorization message id.
pub const AVIATE_ESTIMATOR_STATUS_ID: u32 = 20_000;

/// One parsed frame event from the Aviate subset.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FcMessage {
    /// A private estimator-status frame whose integrity or framing could not
    /// be validated. This event can only revoke authorization.
    InvalidAviateEstimatorStatus,
    /// Link liveness beacon (1 Hz); `armed` is base_mode's
    /// MAV_MODE_FLAG_SAFETY_ARMED bit.
    Heartbeat {
        /// Whether the sender reports itself armed.
        armed: bool,
    },
    /// Command acknowledgement (arm/disarm feedback).
    CommandAck {
        /// The acknowledged MAV_CMD id.
        command: u16,
        /// MAV_RESULT (0 = accepted).
        result: u8,
    },
    /// Attitude estimate (10 Hz): quaternion is body FRD → world NED,
    /// MAVLink order q1=w, q2=x, q3=y, q4=z.
    AttitudeQuaternion {
        /// Milliseconds since FC boot.
        time_boot_ms: u32,
        /// Attitude quaternion (w, x, y, z).
        quat_wxyz: [f32; 4],
        /// Body rates (roll, pitch, yaw) in radians/second.
        rates_rps: [f32; 3],
    },
    /// NED position/velocity estimate (4 Hz).
    LocalPositionNed {
        /// Milliseconds since FC boot.
        time_boot_ms: u32,
        /// Position (north, east, down) in meters.
        pos_ned_m: [f32; 3],
        /// Velocity (north, east, down) in meters/second.
        vel_ned_mps: [f32; 3],
    },
    /// Standard estimator health projection. This message is diagnostic and
    /// never authorizes Pilotage consumers to use an estimate.
    EstimatorStatus {
        /// Microseconds since FC boot.
        time_usec: u64,
        /// MAVLink ESTIMATOR_STATUS flags.
        flags: u16,
    },
    /// Aviate's lossless estimator validity and quality report.
    AviateEstimatorStatus {
        /// Microseconds since FC boot.
        time_usec: u64,
        /// Source validity bits.
        valid_flags: u8,
        /// Source quality enum: 0 unusable, 1 degraded, 2 good.
        quality: u8,
    },
}

/// Datagram parse accounting, for link diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ParseStats {
    /// Frames that decoded to a known message.
    pub decoded: u32,
    /// Frames with a valid layout but a CRC mismatch.
    pub crc_failures: u32,
    /// Private estimator-status frames whose integrity, compatibility, or
    /// declared frame extent could not be validated. Consumers use this as an
    /// immediate revocation signal; it never authorizes numeric telemetry.
    pub invalid_estimator_status_frames: u32,
    /// Structurally valid frames carrying a message id this parser does
    /// not know (skipped whole).
    pub unknown_ids: u32,
    /// Bytes discarded while hunting for a frame start.
    pub garbage_bytes: u32,
}

/// Sender identity retained from one MAVLink frame header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSource {
    /// MAVLink system identifier.
    pub system_id: u8,
    /// MAVLink component identifier within the system.
    pub component_id: u8,
    /// Sender's wrapping frame sequence, retained for diagnostics.
    pub frame_sequence: u8,
}

/// COMMAND_LONG message id (uplink: arm/disarm).
pub const COMMAND_LONG_ID: u32 = 76;
/// SET_POSITION_TARGET_LOCAL_NED message id (uplink: velocity setpoints).
pub const SET_POSITION_TARGET_ID: u32 = 84;

/// CRC_EXTRA per message id, from the MAVLink XML definitions (matching
/// `aviate-link`); `None` for ids this parser cannot verify.
fn crc_extra(msg_id: u32) -> Option<u8> {
    match msg_id {
        HEARTBEAT_ID => Some(50),
        ATTITUDE_QUATERNION_ID => Some(246),
        LOCAL_POSITION_NED_ID => Some(185),
        COMMAND_ACK_ID => Some(143),
        ESTIMATOR_STATUS_ID => Some(163),
        AVIATE_ESTIMATOR_STATUS_ID => Some(171),
        COMMAND_LONG_ID => Some(152),
        SET_POSITION_TARGET_ID => Some(143),
        _ => None,
    }
}

/// One step of the MAVLink X.25 CRC.
fn crc_accumulate(byte: u8, crc: u16) -> u16 {
    let tmp = byte ^ (crc as u8);
    let tmp = tmp ^ (tmp << 4);
    (crc >> 8) ^ (u16::from(tmp) << 8) ^ (u16::from(tmp) << 3) ^ (u16::from(tmp) >> 4)
}

/// MAVLink CRC over `data`, finished with the message's CRC_EXTRA.
fn compute_crc(data: &[u8], extra: u8) -> u16 {
    let mut crc = 0xFFFFu16;
    for &b in data {
        crc = crc_accumulate(b, crc);
    }
    crc_accumulate(extra, crc)
}

fn frame_header(bytes: &[u8], at: usize) -> Option<(FrameSource, u32)> {
    let header = bytes.get(at..at + 10)?;
    let source = FrameSource {
        frame_sequence: header[4],
        system_id: header[5],
        component_id: header[6],
    };
    let message_id =
        u32::from(header[7]) | (u32::from(header[8]) << 8) | (u32::from(header[9]) << 16);
    Some((source, message_id))
}

fn record_invalid_private_status(
    bytes: &[u8],
    at: usize,
    stats: &mut ParseStats,
    out: &mut Vec<(FrameSource, FcMessage)>,
) {
    let Some((source, message_id)) = frame_header(bytes, at) else {
        return;
    };
    if message_id == AVIATE_ESTIMATOR_STATUS_ID {
        stats.invalid_estimator_status_frames =
            stats.invalid_estimator_status_frames.wrapping_add(1);
        out.push((source, FcMessage::InvalidAviateEstimatorStatus));
    }
}

/// Parses every MAVLink 2.0 frame in `bytes` (a UDP datagram may carry
/// several), appending `(sender identity, message)` pairs to `out` and
/// returning parse accounting. Consumers match both system and component ids;
/// first-packet source selection is not an identity policy.
/// Unknown ids and ordinary CRC failures skip the frame. A private-status
/// integrity failure appends a source-tagged revocation event. Stray bytes
/// before a frame start are discarded byte-by-byte.
pub fn parse_datagram(bytes: &[u8], out: &mut Vec<(FrameSource, FcMessage)>) -> ParseStats {
    let mut stats = ParseStats::default();
    let mut at = 0usize;
    while at < bytes.len() {
        if bytes.get(at) != Some(&MAGIC_V2) {
            stats.garbage_bytes = stats.garbage_bytes.wrapping_add(1);
            at = at.saturating_add(1);
            continue;
        }
        // Header: STX len incompat compat seq sysid compid msgid[3].
        let Some(&len) = bytes.get(at + 1) else { break };
        let Some(&incompat) = bytes.get(at + 2) else {
            break;
        };
        if incompat & !0x01 != 0 {
            record_invalid_private_status(bytes, at, &mut stats, out);
            // Unknown incompatibility bits may change the frame layout, so
            // no later byte boundary in this datagram is trustworthy.
            stats.garbage_bytes = stats.garbage_bytes.wrapping_add((bytes.len() - at) as u32);
            break;
        }
        let payload_len = usize::from(len);
        let signed = incompat & 0x01 != 0;
        let sig_len = if signed { 13 } else { 0 };
        let frame_len = 10 + payload_len + 2 + sig_len;
        let Some(frame) = bytes.get(at..at + frame_len) else {
            // Truncated tail; nothing after it can parse either.
            record_invalid_private_status(bytes, at, &mut stats, out);
            stats.garbage_bytes = stats.garbage_bytes.wrapping_add((bytes.len() - at) as u32);
            break;
        };
        let source = FrameSource {
            frame_sequence: frame[4],
            system_id: frame[5],
            component_id: frame[6],
        };
        let msg_id = u32::from(frame[7]) | (u32::from(frame[8]) << 8) | (u32::from(frame[9]) << 16);
        let crc_at = 10 + payload_len;
        let wire_crc = u16::from(frame[crc_at]) | (u16::from(frame[crc_at + 1]) << 8);
        match crc_extra(msg_id) {
            Some(extra) => {
                let computed = compute_crc(&frame[1..crc_at], extra);
                if computed == wire_crc {
                    if let Some(msg) = decode_known(msg_id, &frame[10..crc_at]) {
                        out.push((source, msg));
                        stats.decoded = stats.decoded.wrapping_add(1);
                    }
                } else {
                    stats.crc_failures = stats.crc_failures.wrapping_add(1);
                    if msg_id == AVIATE_ESTIMATOR_STATUS_ID {
                        stats.invalid_estimator_status_frames =
                            stats.invalid_estimator_status_frames.wrapping_add(1);
                        out.push((source, FcMessage::InvalidAviateEstimatorStatus));
                    }
                }
            }
            None => {
                stats.unknown_ids = stats.unknown_ids.wrapping_add(1);
            }
        }
        at = at.saturating_add(frame_len);
    }
    stats
}

/// Writes a MAVLink 2.0 frame (GCS sysid 255, compid 190) around
/// `payload` into `out`, returning the frame length. `out` must hold
/// `10 + payload.len() + 2` bytes; a too-small buffer returns 0.
fn encode_frame_v2(seq: u8, msg_id: u32, payload: &[u8], extra: u8, out: &mut [u8]) -> usize {
    let total = 10 + payload.len() + 2;
    if out.len() < total || payload.len() > 255 {
        return 0;
    }
    out[0] = MAGIC_V2;
    out[1] = payload.len() as u8;
    out[2] = 0;
    out[3] = 0;
    out[4] = seq;
    out[5] = 255;
    out[6] = 190;
    out[7] = (msg_id & 0xff) as u8;
    out[8] = ((msg_id >> 8) & 0xff) as u8;
    out[9] = ((msg_id >> 16) & 0xff) as u8;
    out[10..10 + payload.len()].copy_from_slice(payload);
    let crc = compute_crc(&out[1..10 + payload.len()], extra);
    out[10 + payload.len()] = (crc & 0xff) as u8;
    out[10 + payload.len() + 1] = (crc >> 8) as u8;
    total
}

/// Serializes a GCS heartbeat frame, used to register this endpoint with
/// a MAVLink router that only forwards to peers it has heard from.
pub fn encode_gcs_heartbeat(seq: u8) -> [u8; 21] {
    // Payload: custom_mode u32 (0), type MAV_TYPE_GCS=6, autopilot
    // MAV_AUTOPILOT_INVALID=8, base_mode 0, system_status MAV_STATE_ACTIVE=4,
    // mavlink_version 3.
    let mut payload = [0u8; 9];
    payload[4] = 6;
    payload[5] = 8;
    payload[7] = 4;
    payload[8] = 3;
    let mut frame = [0u8; 21];
    encode_frame_v2(seq, HEARTBEAT_ID, &payload, 50, &mut frame);
    frame
}

/// Serializes a COMMAND_LONG arm/disarm frame (MAV_CMD 400) for the selected
/// MAVLink system and component.
///
/// Wire order (33 bytes): param1..7 f32 @0..28, command u16 @28,
/// target_system @30, target_component @31, confirmation @32.
pub fn encode_arm_command(seq: u8, arm: bool, target_system: u8, target_component: u8) -> [u8; 45] {
    let mut payload = [0u8; 33];
    let param1: f32 = if arm { 1.0 } else { 0.0 };
    payload[0..4].copy_from_slice(&param1.to_le_bytes());
    payload[28..30].copy_from_slice(&400u16.to_le_bytes());
    payload[30] = target_system;
    payload[31] = target_component;
    let mut frame = [0u8; 45];
    encode_frame_v2(seq, COMMAND_LONG_ID, &payload, 152, &mut frame);
    frame
}

/// Serializes a SET_POSITION_TARGET_LOCAL_NED velocity setpoint: NED
/// velocity plus absolute yaw, everything else masked out
/// (`type_mask` ignores position, acceleration, and yaw rate — the
/// combination Aviate's bridge maps to `ControlMode::VelocityControl`).
///
/// Serializes a SET_POSITION_TARGET_LOCAL_NED **position-hold**
/// setpoint: NED position plus absolute yaw, velocity/acceleration/yaw
/// rate masked out (the combination Aviate's bridge maps to
/// `ControlMode::PositionHold`) — DJI's brake-then-hold on centered
/// sticks.
pub fn encode_position_setpoint(
    seq: u8,
    time_boot_ms: u32,
    pos_ned_m: [f32; 3],
    yaw_rad: f32,
    target_system: u8,
    target_component: u8,
) -> [u8; 65] {
    // Ignore velocity (8|16|32), acceleration (64|128|256), yaw rate
    // (2048): position + absolute yaw remain.
    const TYPE_MASK: u16 = 8 | 16 | 32 | 64 | 128 | 256 | 2048;
    let mut payload = [0u8; 53];
    payload[0..4].copy_from_slice(&time_boot_ms.to_le_bytes());
    payload[4..8].copy_from_slice(&pos_ned_m[0].to_le_bytes());
    payload[8..12].copy_from_slice(&pos_ned_m[1].to_le_bytes());
    payload[12..16].copy_from_slice(&pos_ned_m[2].to_le_bytes());
    payload[40..44].copy_from_slice(&yaw_rad.to_le_bytes());
    payload[48..50].copy_from_slice(&TYPE_MASK.to_le_bytes());
    payload[50] = target_system;
    payload[51] = target_component;
    payload[52] = 1; // MAV_FRAME_LOCAL_NED
    let mut frame = [0u8; 65];
    encode_frame_v2(seq, SET_POSITION_TARGET_ID, &payload, 143, &mut frame);
    frame
}

/// Serializes a SET_POSITION_TARGET_LOCAL_NED velocity setpoint: NED
/// velocity plus absolute yaw, everything else masked out
/// (`type_mask` ignores position, acceleration, and yaw rate — the
/// combination Aviate's bridge maps to `ControlMode::VelocityControl`).
///
/// Wire order (53 bytes): time_boot_ms u32 @0, x/y/z @4..16,
/// vx/vy/vz @16..28, afx/afy/afz @28..40, yaw @40, yaw_rate @44,
/// type_mask u16 @48, target_system @50, target_component @51,
/// coordinate_frame @52.
pub fn encode_velocity_setpoint(
    seq: u8,
    time_boot_ms: u32,
    vel_ned_mps: [f32; 3],
    yaw_rad: f32,
    target_system: u8,
    target_component: u8,
) -> [u8; 65] {
    // Ignore position (1|2|4), acceleration (64|128|256), yaw rate (2048):
    // velocity + absolute yaw remain.
    const TYPE_MASK: u16 = 1 | 2 | 4 | 64 | 128 | 256 | 2048;
    let mut payload = [0u8; 53];
    payload[0..4].copy_from_slice(&time_boot_ms.to_le_bytes());
    payload[16..20].copy_from_slice(&vel_ned_mps[0].to_le_bytes());
    payload[20..24].copy_from_slice(&vel_ned_mps[1].to_le_bytes());
    payload[24..28].copy_from_slice(&vel_ned_mps[2].to_le_bytes());
    payload[40..44].copy_from_slice(&yaw_rad.to_le_bytes());
    payload[48..50].copy_from_slice(&TYPE_MASK.to_le_bytes());
    payload[50] = target_system;
    payload[51] = target_component;
    payload[52] = 1; // MAV_FRAME_LOCAL_NED
    let mut frame = [0u8; 65];
    encode_frame_v2(seq, SET_POSITION_TARGET_ID, &payload, 143, &mut frame);
    frame
}

/// Absolute FPV attitude demand and the component selected to receive it.
///
/// Body-rate demand is intentionally absent because the FC attitude loop
/// derives rate commands from this target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttitudeTarget {
    /// Roll demand in radians.
    pub roll_rad: f32,
    /// Pitch demand in radians.
    pub pitch_rad: f32,
    /// Yaw demand in radians.
    pub yaw_rad: f32,
    /// Normalized collective thrust.
    pub thrust: f32,
    /// Selected MAVLink system id.
    pub system_id: u8,
    /// Selected MAVLink component id.
    pub component_id: u8,
}

/// Encodes one absolute attitude target for the selected component.
pub fn encode_attitude_setpoint(seq: u8, time_boot_ms: u32, target: AttitudeTarget) -> [u8; 51] {
    const SET_ATTITUDE_TARGET_ID: u32 = 82;
    let (sr, cr) = (target.roll_rad * 0.5).sin_cos();
    let (sp, cp) = (target.pitch_rad * 0.5).sin_cos();
    let (sy, cy) = (target.yaw_rad * 0.5).sin_cos();
    let q = [
        cr * cp * cy + sr * sp * sy,
        sr * cp * cy - cr * sp * sy,
        cr * sp * cy + sr * cp * sy,
        cr * cp * sy - sr * sp * cy,
    ];
    let mut payload = [0u8; 39];
    payload[0..4].copy_from_slice(&time_boot_ms.to_le_bytes());
    for (i, w) in q.iter().enumerate() {
        payload[4 + i * 4..8 + i * 4].copy_from_slice(&w.to_le_bytes());
    }
    // body rates [20..32) stay zero (masked); thrust at [32..36).
    payload[32..36].copy_from_slice(&target.thrust.clamp(0.0, 1.0).to_le_bytes());
    payload[36] = target.system_id;
    payload[37] = target.component_id;
    payload[38] = 0b0000_0111; // ignore body roll/pitch/yaw rate
    let mut frame = [0u8; 51];
    encode_frame_v2(seq, SET_ATTITUDE_TARGET_ID, &payload, 49, &mut frame);
    frame
}

#[cfg(test)]
mod tests;
