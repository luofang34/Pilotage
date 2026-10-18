//! MANUAL_CONTROL, PARAM_SET, PARAM_VALUE, and the heartbeat flight mode
//! against frames that the MAVLink C library (PX4's generated common-dialect
//! headers) packed, so offsets and CRC_EXTRA values are checked against an
//! independent implementation.

#![allow(clippy::panic)]

use super::super::{
    FcMessage, FrameSource, ManualControlSticks, ParamValue, encode_manual_control,
    encode_param_set, param_id, parse_datagram,
};

/// x = -250, y = 500, z = 750, r = 0, no buttons, target system 7, seq 3.
const MANUAL_CONTROL: [u8; 23] = [
    0xfd, 0x0b, 0x00, 0x00, 0x03, 0xff, 0xbe, 0x45, 0x00, 0x00, 0x06, 0xff, 0xf4, 0x01, 0xee, 0x02,
    0x00, 0x00, 0x00, 0x00, 0x07, 0xd3, 0x94,
];

/// FW_AIRSPD_TRIM = 15.0 (REAL32) to system 7 component 1, seq 4.
const PARAM_SET_REAL: [u8; 35] = [
    0xfd, 0x17, 0x00, 0x00, 0x04, 0xff, 0xbe, 0x17, 0x00, 0x00, 0x00, 0x00, 0x70, 0x41, 0x07, 0x01,
    0x46, 0x57, 0x5f, 0x41, 0x49, 0x52, 0x53, 0x50, 0x44, 0x5f, 0x54, 0x52, 0x49, 0x4d, 0x00, 0x00,
    0x09, 0x2a, 0x25,
];

/// COM_RC_IN_MODE = 1 (INT32, bytewise) to system 7 component 1, seq 5.
const PARAM_SET_INT: [u8; 35] = [
    0xfd, 0x17, 0x00, 0x00, 0x05, 0xff, 0xbe, 0x17, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x07, 0x01,
    0x43, 0x4f, 0x4d, 0x5f, 0x52, 0x43, 0x5f, 0x49, 0x4e, 0x5f, 0x4d, 0x4f, 0x44, 0x45, 0x00, 0x00,
    0x06, 0xc5, 0x26,
];

/// NAV_RCL_ACT = 1 (INT32, bytewise) from system 7 component 1.
const PARAM_VALUE_INT: [u8; 37] = [
    0xfd, 0x19, 0x00, 0x00, 0x06, 0x07, 0x01, 0x16, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x4c, 0x04,
    0x2a, 0x00, 0x4e, 0x41, 0x56, 0x5f, 0x52, 0x43, 0x4c, 0x5f, 0x41, 0x43, 0x54, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x06, 0x00, 0x03,
];

/// An armed fixed-wing in PX4 Position mode (main mode 3).
const HEARTBEAT_POSCTL_ARMED: [u8; 21] = [
    0xfd, 0x09, 0x00, 0x00, 0x08, 0x07, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x01, 0x0c,
    0x81, 0x04, 0x03, 0xe7, 0xe2,
];

const PX4_SYSTEM_7: FrameSource = FrameSource {
    system_id: 7,
    component_id: 1,
    frame_sequence: 0,
};

#[test]
fn manual_control_matches_the_c_library_frame() {
    let sticks = ManualControlSticks {
        x: -250,
        y: 500,
        z: 750,
        r: 0,
    };
    assert_eq!(encode_manual_control(3, sticks, 7), MANUAL_CONTROL);
}

#[test]
fn manual_control_clamps_out_of_range_sticks_to_the_wire_range() {
    let wild = ManualControlSticks {
        x: 3000,
        y: -3000,
        z: -5,
        r: i16::MIN,
    };
    let frame = encode_manual_control(0, wild, 1);
    let read = |at: usize| i16::from_le_bytes([frame[10 + at], frame[11 + at]]);
    assert_eq!(
        (read(0), read(2), read(4), read(6)),
        (1000, -1000, 0, -1000)
    );
}

#[test]
fn param_set_matches_the_c_library_frames() {
    assert_eq!(
        encode_param_set(4, "FW_AIRSPD_TRIM", ParamValue::Real32(15.0), 7, 1),
        PARAM_SET_REAL
    );
    assert_eq!(
        encode_param_set(5, "COM_RC_IN_MODE", ParamValue::Int32(1), 7, 1),
        PARAM_SET_INT
    );
}

#[test]
fn param_value_decodes_an_integer_bytewise() {
    let mut out = Vec::new();
    let stats = parse_datagram(&PARAM_VALUE_INT, &mut out);
    assert_eq!(stats.decoded, 1, "stats: {stats:?}");
    let Some((
        source,
        FcMessage::ParamValue {
            param_id: id,
            value,
        },
    )) = out.first().copied()
    else {
        panic!("expected one PARAM_VALUE, got {out:?}");
    };
    assert_eq!((source.system_id, source.component_id), (7, 1));
    assert_eq!(id, param_id("NAV_RCL_ACT"));
    assert_eq!(value, ParamValue::Int32(1));
}

#[test]
fn heartbeat_carries_the_custom_flight_mode() {
    let mut out = Vec::new();
    parse_datagram(&HEARTBEAT_POSCTL_ARMED, &mut out);
    assert_eq!(
        out,
        vec![(
            FrameSource {
                frame_sequence: 8,
                ..PX4_SYSTEM_7
            },
            FcMessage::Heartbeat {
                armed: true,
                custom_mode: 3 << 16,
            }
        )]
    );
}

#[test]
fn param_names_are_nul_padded_and_truncated_at_sixteen_bytes() {
    assert_eq!(&param_id("FW_R_LIM")[..9], b"FW_R_LIM\0");
    assert_eq!(&param_id("A_NAME_LONGER_THAN_16"), b"A_NAME_LONGER_TH");
}

#[test]
fn fc_side_param_value_and_heartbeat_parse_back() {
    let mut datagram =
        super::super::encode_param_value(1, "FW_R_LIM", ParamValue::Real32(50.0), 7, 1).to_vec();
    datagram.extend_from_slice(&super::super::encode_autopilot_heartbeat(
        2,
        true,
        3 << 16,
        7,
        1,
    ));
    let mut out = Vec::new();
    let stats = parse_datagram(&datagram, &mut out);
    assert_eq!(stats.decoded, 2, "stats: {stats:?}");
    let messages: Vec<FcMessage> = out.iter().map(|(_, message)| *message).collect();
    assert_eq!(
        messages,
        vec![
            FcMessage::ParamValue {
                param_id: param_id("FW_R_LIM"),
                value: ParamValue::Real32(50.0),
            },
            FcMessage::Heartbeat {
                armed: true,
                custom_mode: 3 << 16,
            },
        ]
    );
}
