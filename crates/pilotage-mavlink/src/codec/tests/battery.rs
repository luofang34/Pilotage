//! BATTERY_STATUS against frames that pymavlink encoded, so the offsets and
//! the CRC_EXTRA are checked against an independent implementation.

#![allow(clippy::panic)]

use super::super::{BATTERY_STATUS_ID, FcMessage, FrameSource, parse_datagram};

const PYMAVLINK: FrameSource = FrameSource {
    system_id: 1,
    component_id: 1,
    frame_sequence: 0,
};

/// Pack 16.2 V in cell 0, 12.5 A, 4321 hJ used, 73 %, 900 s left.
const KNOWN: [u8; 53] = [
    0xfd, 0x29, 0x00, 0x00, 0x00, 0x01, 0x01, 0x93, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xe1, 0x10,
    0x00, 0x00, 0xff, 0x7f, 0x48, 0x3f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xe2, 0x04, 0x00, 0x00, 0x00, 0x49, 0x84, 0x03,
    0x00, 0x00, 0x01, 0x71, 0x21,
];

/// Every field at its "unknown" value, instance 1.
const UNKNOWN: [u8; 48] = [
    0xfd, 0x24, 0x00, 0x00, 0x00, 0x01, 0x01, 0x93, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01, 0x00, 0x00, 0xff, 0xf3, 0xd4,
];

/// Four cells, zero current and energy, 0 % remaining. The sender trims the
/// trailing zero bytes, so the remaining percent is not on the wire.
const TRIMMED_CELLS: [u8; 45] = [
    0xfd, 0x21, 0x00, 0x00, 0x00, 0x01, 0x01, 0x93, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00,
    0x00, 0x00, 0xff, 0x7f, 0xa0, 0x0f, 0xaa, 0x0f, 0xb4, 0x0f, 0xbe, 0x0f, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x02, 0x08, 0x34,
];

/// Ten cells of 4.0 V and two extension cells, charging at 5 A, 0 %, 60 s.
const TWELVE_CELLS: [u8; 57] = [
    0xfd, 0x2d, 0x00, 0x00, 0x00, 0x01, 0x01, 0x93, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0x7f, 0xa0, 0x0f, 0xa0, 0x0f, 0xa0, 0x0f, 0xa0, 0x0f, 0xa0, 0x0f, 0xa0, 0x0f,
    0xa0, 0x0f, 0xa0, 0x0f, 0xa0, 0x0f, 0xa0, 0x0f, 0x0c, 0xfe, 0x00, 0x00, 0x00, 0x00, 0x3c, 0x00,
    0x00, 0x00, 0x00, 0x04, 0x10, 0x68, 0x10, 0x8f, 0x3b,
];

/// A remaining percent of 120, which the definition does not allow.
const OVER_FULL: [u8; 48] = [
    0xfd, 0x24, 0x00, 0x00, 0x00, 0x01, 0x01, 0x93, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x78, 0x73, 0x1e,
];

fn decode(frame: &[u8]) -> Vec<(FrameSource, FcMessage)> {
    let mut out = Vec::new();
    let stats = parse_datagram(frame, &mut out);
    assert_eq!(stats.crc_failures, 0);
    assert_eq!(stats.unknown_ids, 0);
    out
}

#[test]
fn a_known_battery_report_decodes_in_wire_units() {
    assert_eq!(u32::from(KNOWN[7]), BATTERY_STATUS_ID);
    assert_eq!(
        decode(&KNOWN),
        vec![(
            PYMAVLINK,
            FcMessage::BatteryStatus {
                instance: 0,
                remaining_percent: Some(73),
                voltage_mv: Some(16_200),
                current_ca: Some(1_250),
                energy_consumed_hj: Some(4_321),
                time_remaining_s: Some(900),
            }
        )]
    );
}

#[test]
fn unknown_values_decode_as_none() {
    assert_eq!(
        decode(&UNKNOWN),
        vec![(
            PYMAVLINK,
            FcMessage::BatteryStatus {
                instance: 1,
                remaining_percent: None,
                voltage_mv: None,
                current_ca: None,
                energy_consumed_hj: None,
                time_remaining_s: None,
            }
        )]
    );
}

#[test]
fn cells_sum_to_the_pack_and_a_trimmed_zero_is_a_real_zero() {
    assert_eq!(
        decode(&TRIMMED_CELLS),
        vec![(
            PYMAVLINK,
            FcMessage::BatteryStatus {
                instance: 2,
                remaining_percent: Some(0),
                voltage_mv: Some(16_060),
                current_ca: Some(0),
                energy_consumed_hj: Some(0),
                time_remaining_s: None,
            }
        )]
    );
}

#[test]
fn a_corrupted_battery_frame_is_refused_by_its_crc() {
    let mut frame = KNOWN;
    frame[45] ^= 0x01;
    let mut out = Vec::new();
    let stats = parse_datagram(&frame, &mut out);
    assert!(out.is_empty());
    assert_eq!(stats.crc_failures, 1);
}

#[test]
fn extension_cells_add_to_the_pack_voltage() {
    assert_eq!(
        decode(&TWELVE_CELLS),
        vec![(
            PYMAVLINK,
            FcMessage::BatteryStatus {
                instance: 0,
                remaining_percent: Some(0),
                voltage_mv: Some(48_300),
                current_ca: Some(-500),
                energy_consumed_hj: None,
                time_remaining_s: Some(60),
            }
        )]
    );
}

#[test]
fn a_remaining_percent_above_full_is_unknown() {
    let out = decode(&OVER_FULL);
    let [
        (
            _,
            FcMessage::BatteryStatus {
                remaining_percent, ..
            },
        ),
    ] = out.as_slice()
    else {
        panic!("one battery report");
    };
    assert_eq!(*remaining_percent, None);
}
