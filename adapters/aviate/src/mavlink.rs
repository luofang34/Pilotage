//! Minimal MAVLink 2.0 frame math for Aviate's telemetry subset
//! (ADR-0018): pure byte functions, no I/O, no allocation on the parse
//! path beyond the caller's message vector.
//!
//! Only the three message ids Aviate emits are decoded; anything else is
//! counted and skipped. CRC is the MAVLink X.25 accumulator seeded with
//! each message's CRC_EXTRA (values cross-checked against Aviate's
//! `aviate-link` implementation, the peer this parser must interoperate
//! with byte-for-byte).

/// MAVLink 2.0 start-of-frame marker.
pub const MAGIC_V2: u8 = 0xFD;
/// HEARTBEAT message id.
pub const HEARTBEAT_ID: u32 = 0;
/// ATTITUDE_QUATERNION message id.
pub const ATTITUDE_QUATERNION_ID: u32 = 31;
/// LOCAL_POSITION_NED message id.
pub const LOCAL_POSITION_NED_ID: u32 = 32;

/// One decoded telemetry message from the Aviate subset.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AviateMessage {
    /// Link liveness beacon (1 Hz).
    Heartbeat,
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
}

/// Datagram parse accounting, for link diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ParseStats {
    /// Frames that decoded to a known message.
    pub decoded: u32,
    /// Frames with a valid layout but a CRC mismatch.
    pub crc_failures: u32,
    /// Structurally valid frames carrying a message id this parser does
    /// not know (skipped whole).
    pub unknown_ids: u32,
    /// Bytes discarded while hunting for a frame start.
    pub garbage_bytes: u32,
}

/// CRC_EXTRA per message id, from the MAVLink XML definitions (matching
/// `aviate-link`); `None` for ids this parser cannot verify.
fn crc_extra(msg_id: u32) -> Option<u8> {
    match msg_id {
        HEARTBEAT_ID => Some(50),
        ATTITUDE_QUATERNION_ID => Some(246),
        LOCAL_POSITION_NED_ID => Some(185),
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

/// Reads an `f32` at `off`, zero-extending past the payload end
/// (MAVLink 2 truncates trailing zero bytes on the wire).
fn f32_at(payload: &[u8], off: usize) -> f32 {
    let mut b = [0u8; 4];
    for (i, slot) in b.iter_mut().enumerate() {
        *slot = payload.get(off + i).copied().unwrap_or(0);
    }
    f32::from_le_bytes(b)
}

/// Reads a `u32` at `off` with the same zero extension.
fn u32_at(payload: &[u8], off: usize) -> u32 {
    let mut b = [0u8; 4];
    for (i, slot) in b.iter_mut().enumerate() {
        *slot = payload.get(off + i).copied().unwrap_or(0);
    }
    u32::from_le_bytes(b)
}

fn decode_known(msg_id: u32, payload: &[u8]) -> Option<AviateMessage> {
    match msg_id {
        HEARTBEAT_ID => Some(AviateMessage::Heartbeat),
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
        _ => None,
    }
}

/// Parses every MAVLink 2.0 frame in `bytes` (a UDP datagram may carry
/// several), appending decoded messages to `out` and returning parse
/// accounting. Unknown ids and CRC failures skip the frame; stray bytes
/// before a frame start are discarded byte-by-byte.
pub fn parse_datagram(bytes: &[u8], out: &mut Vec<AviateMessage>) -> ParseStats {
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
        let payload_len = usize::from(len);
        let signed = incompat & 0x01 != 0;
        let sig_len = if signed { 13 } else { 0 };
        let frame_len = 10 + payload_len + 2 + sig_len;
        let Some(frame) = bytes.get(at..at + frame_len) else {
            // Truncated tail; nothing after it can parse either.
            stats.garbage_bytes = stats.garbage_bytes.wrapping_add((bytes.len() - at) as u32);
            break;
        };
        let msg_id = u32::from(frame[7]) | (u32::from(frame[8]) << 8) | (u32::from(frame[9]) << 16);
        let crc_at = 10 + payload_len;
        let wire_crc = u16::from(frame[crc_at]) | (u16::from(frame[crc_at + 1]) << 8);
        match crc_extra(msg_id) {
            Some(extra) => {
                let computed = compute_crc(&frame[1..crc_at], extra);
                if computed == wire_crc {
                    if let Some(msg) = decode_known(msg_id, &frame[10..crc_at]) {
                        out.push(msg);
                        stats.decoded = stats.decoded.wrapping_add(1);
                    }
                } else {
                    stats.crc_failures = stats.crc_failures.wrapping_add(1);
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

/// Serializes a GCS heartbeat frame, used to register this endpoint with
/// a MAVLink router that only forwards to peers it has heard from.
pub fn encode_gcs_heartbeat(seq: u8) -> [u8; 21] {
    let mut frame = [0u8; 21];
    frame[0] = MAGIC_V2;
    frame[1] = 9; // payload length
    // incompat/compat = 0; seq; sysid 255 (GCS convention); compid 190.
    frame[4] = seq;
    frame[5] = 255;
    frame[6] = 190;
    // msgid 0 (HEARTBEAT) is already zero.
    // Payload: custom_mode u32 (0), type MAV_TYPE_GCS=6, autopilot
    // MAV_AUTOPILOT_INVALID=8, base_mode 0, system_status MAV_STATE_ACTIVE=4,
    // mavlink_version 3.
    frame[14] = 6;
    frame[15] = 8;
    frame[17] = 4;
    frame[18] = 3;
    let crc = compute_crc(&frame[1..19], 50);
    frame[19] = (crc & 0xff) as u8;
    frame[20] = (crc >> 8) as u8;
    frame
}

#[cfg(test)]
mod tests;
