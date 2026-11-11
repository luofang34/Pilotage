//! VFR_HUD against a frame pymavlink 2.4.41 encoded (the golden frame
//! Aviate's own codec is pinned to), and the airspeeds that read as none.

#![allow(clippy::panic)]

use super::super::{FcMessage, VFR_HUD_ID, VfrHud, parse_datagram};
use super::encode_frame;

/// Airspeed 52.5 m/s, groundspeed 48.25, altitude 321.5, climb -1.75,
/// heading 275, throttle 84; the zero high byte of the throttle trims the
/// payload to 19 bytes.
const PYMAVLINK_VFR_HUD: [u8; 31] = [
    0xFD, 0x13, 0x00, 0x00, 0x09, 0x01, 0x01, 0x4A, 0x00, 0x00, 0x00, 0x00, 0x52, 0x42, 0x00, 0x00,
    0x41, 0x42, 0x00, 0xC0, 0xA0, 0x43, 0x00, 0x00, 0xE0, 0xBF, 0x13, 0x01, 0x54, 0xB4, 0x49,
];

fn airspeed_of(frame: &[u8]) -> Option<f32> {
    let mut out = Vec::new();
    let stats = parse_datagram(frame, &mut out);
    assert_eq!((stats.crc_failures, stats.unknown_ids), (0, 0), "{stats:?}");
    match out.as_slice() {
        [
            (
                _,
                FcMessage::VfrHud(VfrHud {
                    indicated_airspeed_mps,
                }),
            ),
        ] => *indicated_airspeed_mps,
        other => panic!("one VFR_HUD expected, got {other:?}"),
    }
}

fn hud_payload(airspeed: f32) -> Vec<u8> {
    let mut payload = airspeed.to_le_bytes().to_vec();
    payload.extend_from_slice(&[0; 16]);
    payload
}

#[test]
fn the_airspeed_is_read_from_a_pymavlink_frame() {
    assert_eq!(airspeed_of(&PYMAVLINK_VFR_HUD), Some(52.5));
}

#[test]
fn a_nan_or_negative_airspeed_is_none() {
    assert_eq!(
        airspeed_of(&encode_frame(VFR_HUD_ID, &hud_payload(f32::NAN), true)),
        None
    );
    assert_eq!(
        airspeed_of(&encode_frame(VFR_HUD_ID, &hud_payload(-3.0), true)),
        None
    );
    assert_eq!(
        airspeed_of(&encode_frame(VFR_HUD_ID, &hud_payload(0.0), true)),
        Some(0.0)
    );
}
