//! The truth lane's own frames: the barometric and ground-truth payloads
//! the link reads, and what a report short of its position decodes to.

#![allow(clippy::expect_used, clippy::panic)]

use super::encode_frame;
use crate::codec::{FcMessage, HIL_STATE_QUATERNION_ID, SCALED_PRESSURE_ID, parse_datagram};

#[test]
fn decodes_scaled_pressure_and_sim_truth() {
    // SCALED_PRESSURE: time u32, press_abs f32 (hPa), press_diff f32,
    // temperature i16 — standard-datum sea level, 25.00 °C.
    let mut pressure = Vec::new();
    pressure.extend_from_slice(&1000_u32.to_le_bytes());
    pressure.extend_from_slice(&1013.25_f32.to_le_bytes());
    pressure.extend_from_slice(&0.0_f32.to_le_bytes());
    pressure.extend_from_slice(&2500_i16.to_le_bytes());
    let mut datagram = encode_frame(SCALED_PRESSURE_ID, &pressure, true);

    // HIL_STATE_QUATERNION with identity attitude, 1 m/s north, and a
    // fix at 47.0°/8.0°/500 m.
    let mut truth = vec![0_u8; 64];
    truth[0..8].copy_from_slice(&2_000_000_u64.to_le_bytes());
    truth[8..12].copy_from_slice(&1.0_f32.to_le_bytes());
    truth[36..40].copy_from_slice(&470_000_000_i32.to_le_bytes());
    truth[40..44].copy_from_slice(&80_000_000_i32.to_le_bytes());
    truth[44..48].copy_from_slice(&500_000_i32.to_le_bytes());
    truth[48..50].copy_from_slice(&100_i16.to_le_bytes());
    datagram.extend_from_slice(&encode_frame(HIL_STATE_QUATERNION_ID, &truth, true));

    let mut out = Vec::new();
    let stats = parse_datagram(&datagram, &mut out);
    assert_eq!(stats.crc_failures, 0, "both new ids must verify");
    assert_eq!(stats.decoded, 2, "both frames decode: {out:?}");
    assert!(matches!(
        out[0].1,
        FcMessage::ScaledPressure { press_abs_hpa, .. } if (press_abs_hpa - 1013.25).abs() < 0.01
    ));
    assert!(matches!(
        out[1].1,
        FcMessage::SimTruth { vel_ned_mps, .. } if (vel_ned_mps[0] - 1.0).abs() < 0.01
    ));
    // The geodetic triple is read at fixed offsets and every position the
    // map draws comes from them. An offset read one field early decodes a
    // plausible place that is not the one the simulator stated, and the
    // velocity assertion above does not move.
    assert!(matches!(
        out[1].1,
        FcMessage::SimTruth { lat_lon_alt, .. }
            if lat_lon_alt == [470_000_000, 80_000_000, 500_000]
    ));
}

/// MAVLink 2 trims trailing zero bytes and every reader zero-extends them
/// back. That trimming can cut into the middle of a value whose high bytes
/// are zero, so no offset can be required to be present: a vehicle at rest
/// at 47N 8E and 500 m sends a report whose acceleration and velocity
/// fields are zero and whose altitude's most significant byte is zero, and
/// a length gate at the end of the geodetic triple drops the whole frame —
/// costing the attitude and the velocity along with the position.
#[test]
fn a_truth_report_trimmed_to_its_last_meaning_byte_still_decodes() {
    let mut payload = vec![0_u8; 64];
    payload[0..8].copy_from_slice(&2_000_000_u64.to_le_bytes());
    payload[8..12].copy_from_slice(&1.0_f32.to_le_bytes());
    payload[36..40].copy_from_slice(&473_977_419_i32.to_le_bytes());
    payload[40..44].copy_from_slice(&85_455_938_i32.to_le_bytes());
    payload[44..48].copy_from_slice(&500_000_i32.to_le_bytes());
    // 500_000 millimetres is 0x0007_A120: its most significant byte is
    // zero, so a real sender trims the frame to 47 bytes.
    let datagram = encode_frame(HIL_STATE_QUATERNION_ID, &payload, true);

    let mut out = Vec::new();
    let stats = parse_datagram(&datagram, &mut out);
    assert_eq!(stats.crc_failures, 0, "the trimmed frame verifies");
    assert_eq!(out.len(), 1, "a vehicle at rest is still a report: {out:?}");
    assert!(matches!(
        out[0].1,
        FcMessage::SimTruth { lat_lon_alt, .. }
            if lat_lon_alt == [473_977_419, 85_455_938, 500_000]
    ));
}
