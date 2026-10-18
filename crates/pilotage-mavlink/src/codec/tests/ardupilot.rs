//! ArduPilot's frames against frames that pymavlink (ardupilotmega
//! dialect) encoded, so the offsets and the CRC_EXTRA values are checked
//! against an independent implementation.

#![allow(clippy::panic)]

use super::super::{
    FcMessage, FrameSource, MAV_AUTOPILOT_ARDUPILOTMEGA, encode_body_velocity_setpoint,
    parse_datagram,
};
use crate::ardupilot::ArduCopterMode;

const ARDUPILOT: FrameSource = FrameSource {
    system_id: 1,
    component_id: 1,
    frame_sequence: 0,
};

/// EKF_STATUS_REPORT, flags 0x033f, the zero airspeed variance trimmed.
const EKF_STATUS: [u8; 34] = [
    0xfd, 0x16, 0x00, 0x00, 0x00, 0x01, 0x01, 0xc1, 0x00, 0x00, 0xcd, 0xcc, 0xcc, 0x3d, 0xcd, 0xcc,
    0x4c, 0x3e, 0x9a, 0x99, 0x99, 0x3e, 0xcd, 0xcc, 0xcc, 0x3e, 0x00, 0x00, 0x00, 0x00, 0x3f, 0x03,
    0x88, 0x7e,
];

/// STATUSTEXT, severity 4, "PreArm: Need Position Estimate", trimmed.
const PREARM_TEXT: [u8; 43] = [
    0xfd, 0x1f, 0x00, 0x00, 0x00, 0x01, 0x01, 0xfd, 0x00, 0x00, 0x04, 0x50, 0x72, 0x65, 0x41, 0x72,
    0x6d, 0x3a, 0x20, 0x4e, 0x65, 0x65, 0x64, 0x20, 0x50, 0x6f, 0x73, 0x69, 0x74, 0x69, 0x6f, 0x6e,
    0x20, 0x45, 0x73, 0x74, 0x69, 0x6d, 0x61, 0x74, 0x65, 0x4a, 0xa8,
];

/// HEARTBEAT from an armed ArduCopter quad in GUIDED.
const GUIDED_HEARTBEAT: [u8; 21] = [
    0xfd, 0x09, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x02, 0x03,
    0x99, 0x04, 0x03, 0x7a, 0x70,
];

/// SET_POSITION_TARGET_LOCAL_NED from 255/190, sequence 7: frame 9,
/// type_mask 0x05c7, velocity (1.5, -0.5, -1.0), yaw rate 0.25.
const BODY_VELOCITY: [u8; 65] = [
    0xfd, 0x35, 0x00, 0x00, 0x07, 0xff, 0xbe, 0x54, 0x00, 0x00, 0xd2, 0x04, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc0, 0x3f, 0x00, 0x00,
    0x00, 0xbf, 0x00, 0x00, 0x80, 0xbf, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x3e, 0xc7, 0x05, 0x01, 0x01, 0x09, 0x26,
    0x02,
];

fn decode_one(frame: &[u8]) -> FcMessage {
    let mut out = Vec::new();
    let stats = parse_datagram(frame, &mut out);
    assert_eq!(stats.decoded, 1, "stats: {stats:?}");
    let [(source, message)] = out.as_slice() else {
        panic!("one message, got {out:?}");
    };
    assert_eq!(*source, ARDUPILOT);
    *message
}

#[test]
fn ekf_status_report_decodes_its_flags_from_a_trimmed_frame() {
    assert_eq!(
        decode_one(&EKF_STATUS),
        FcMessage::EkfStatusReport { flags: 0x033f }
    );
}

#[test]
fn status_text_decodes_severity_and_text() {
    let FcMessage::StatusText { severity, text } = decode_one(&PREARM_TEXT) else {
        panic!("a status text");
    };
    assert_eq!(severity, 4);
    let end = text.iter().position(|b| *b == 0).unwrap_or(text.len());
    assert_eq!(&text[..end], b"PreArm: Need Position Estimate");
}

#[test]
fn heartbeat_carries_the_ardupilot_mode() {
    let FcMessage::Heartbeat {
        armed,
        custom_mode,
        autopilot,
    } = decode_one(&GUIDED_HEARTBEAT)
    else {
        panic!("a heartbeat");
    };
    assert!(armed);
    assert_eq!(autopilot, MAV_AUTOPILOT_ARDUPILOTMEGA);
    assert_eq!(
        ArduCopterMode::from_custom_mode(custom_mode),
        ArduCopterMode::Guided
    );
}

#[test]
fn body_velocity_setpoint_matches_the_reference_encoder() {
    let frame = encode_body_velocity_setpoint(7, 1234, [1.5, -0.5, -1.0], 0.25, 1, 1);
    assert_eq!(frame, BODY_VELOCITY);
}
