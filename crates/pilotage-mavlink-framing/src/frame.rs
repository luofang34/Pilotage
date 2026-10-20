use crate::compute_crc;

/// MAVLink 2 start-of-frame marker.
pub const MAGIC_V2: u8 = 0xFD;

/// Writes a MAVLink 2 frame with the caller's source identity.
///
/// Returns the frame length. A payload larger than 255 bytes or an output
/// buffer shorter than `payload.len() + 12` returns zero without writing.
/// The caller supplies the message's CRC_EXTRA value.
#[allow(clippy::too_many_arguments)]
pub fn encode_frame_v2_from(
    seq: u8,
    msg_id: u32,
    payload: &[u8],
    extra: u8,
    system_id: u8,
    component_id: u8,
    out: &mut [u8],
) -> usize {
    let total = 10 + payload.len() + 2;
    if out.len() < total || payload.len() > 255 {
        return 0;
    }
    out[0] = MAGIC_V2;
    out[1] = payload.len() as u8;
    out[2] = 0;
    out[3] = 0;
    out[4] = seq;
    out[5] = system_id;
    out[6] = component_id;
    out[7] = (msg_id & 0xff) as u8;
    out[8] = ((msg_id >> 8) & 0xff) as u8;
    out[9] = ((msg_id >> 16) & 0xff) as u8;
    out[10..10 + payload.len()].copy_from_slice(payload);
    let crc = compute_crc(&out[1..10 + payload.len()], extra);
    out[10 + payload.len()] = (crc & 0xff) as u8;
    out[10 + payload.len() + 1] = (crc >> 8) as u8;
    total
}

#[cfg(test)]
mod tests;
