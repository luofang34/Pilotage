//! Uplink setpoint wire layouts, and the ATTITUDE_TARGET report, against
//! frames the MAVLink C library (PX4's generated common-dialect headers)
//! packed where an independent encoder is available.

#![allow(clippy::panic)]

use super::super::{
    AttitudeTargetKind, FcMessage, MAGIC_V2, encode_attitude_target, parse_datagram,
};

/// Body rates 0.5 / -0.25 / 1.0 rad/s, thrust 0.62, attitude ignored
/// (type_mask 128), identity quaternion, to system 7 component 1, seq 9.
const SET_ATTITUDE_TARGET_RATES: [u8; 51] = [
    0xfd, 0x27, 0x00, 0x00, 0x09, 0xff, 0xbe, 0x52, 0x00, 0x00, 0xd2, 0x04, 0x00, 0x00, 0x00, 0x00,
    0x80, 0x3f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x3f, 0x00, 0x00, 0x80, 0xbe, 0x00, 0x00, 0x80, 0x3f, 0x52, 0xb8, 0x1e, 0x3f, 0x07, 0x01,
    0x80, 0xaf, 0x50,
];

/// PX4's report of its commanded attitude and thrust 0.71, system 7.
const ATTITUDE_TARGET: [u8; 49] = [
    0xfd, 0x25, 0x00, 0x00, 0x0a, 0x07, 0x01, 0x53, 0x00, 0x00, 0x2e, 0x16, 0x00, 0x00, 0x00, 0x00,
    0x80, 0x3f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x8f, 0xc2, 0x35, 0x3f, 0x07, 0x72,
    0x1a,
];

#[test]
fn body_rate_setpoint_matches_the_c_library_frame() {
    let rates = AttitudeTargetKind::BodyRates([0.5, -0.25, 1.0]);
    assert_eq!(
        encode_attitude_target(9, 1234, rates, 0.62, 7, 1),
        SET_ATTITUDE_TARGET_RATES
    );
}

#[test]
fn body_rate_thrust_is_clamped_to_the_normalized_range() {
    let rates = AttitudeTargetKind::BodyRates([0.0; 3]);
    let frame = encode_attitude_target(0, 0, rates, 1.7, 1, 1);
    let thrust = f32::from_le_bytes(frame[42..46].try_into().expect("four bytes"));
    assert_eq!(thrust, 1.0);
}

#[test]
fn euler_attitude_target_ignores_the_body_rates() {
    let level = AttitudeTargetKind::from_euler(0.0, 0.0, 0.0);
    assert_eq!(level, AttitudeTargetKind::Attitude([1.0, 0.0, 0.0, 0.0]));
    let frame = encode_attitude_target(0, 0, level, f32::NAN, 1, 1);
    let thrust = f32::from_le_bytes(frame[42..46].try_into().expect("four bytes"));
    assert_eq!(thrust, 0.0, "NaN thrust is sent as zero");
    assert_eq!(frame[48], 0b0000_0111, "body-rate ignore bits");
}

#[test]
fn attitude_target_reports_the_commanded_thrust() {
    let mut out = Vec::new();
    let stats = parse_datagram(&ATTITUDE_TARGET, &mut out);
    assert_eq!(stats.decoded, 1, "stats: {stats:?}");
    let Some((
        source,
        FcMessage::AttitudeTarget {
            time_boot_ms,
            thrust,
        },
    )) = out.first().copied()
    else {
        panic!("expected one ATTITUDE_TARGET, got {out:?}");
    };
    assert_eq!((source.system_id, time_boot_ms), (7, 5678));
    assert_eq!(thrust, 0.71);
}

#[test]
fn gimbal_rate_setpoint_locks_the_wire_layout() {
    let frame = super::super::encode_gimbal_rate_setpoint(9, 12, [f32::NAN, 0.4, -0.6], 1, 1, 0);
    // Header: v2 magic, 35-byte payload, GCS identity, msg id 282.
    assert_eq!(frame[0], MAGIC_V2);
    assert_eq!(frame[1], 35);
    assert_eq!(frame[4], 9);
    assert_eq!(frame[5], 255);
    assert_eq!(frame[6], 190);
    assert_eq!(
        u32::from(frame[7]) | (u32::from(frame[8]) << 8) | (u32::from(frame[9]) << 16),
        super::super::GIMBAL_MANAGER_SET_ATTITUDE_ID
    );
    let payload = &frame[10..45];
    assert_eq!(
        &payload[0..4],
        &12u32.to_le_bytes(),
        "horizon + yaw-follow flags"
    );
    // q[0..4] and angular_velocity_x are NaN ("ignored").
    for slot in 0..5 {
        let at = 4 + slot * 4;
        let bytes: [u8; 4] = payload[at..at + 4].try_into().expect("slice length");
        assert!(
            f32::from_le_bytes(bytes).is_nan(),
            "slot {slot} must be NaN"
        );
    }
    let pitch: [u8; 4] = payload[24..28].try_into().expect("slice length");
    let yaw: [u8; 4] = payload[28..32].try_into().expect("slice length");
    assert_eq!(f32::from_le_bytes(pitch), 0.4);
    assert_eq!(f32::from_le_bytes(yaw), -0.6);
    assert_eq!(payload[32], 1, "target system");
    assert_eq!(payload[33], 1, "target component");
    assert_eq!(payload[34], 0, "all gimbal devices");
    // The CRC must verify under CRC_EXTRA 123 (checked against PX4's
    // generated common-dialect headers).
    let wire_crc = u16::from(frame[45]) | (u16::from(frame[46]) << 8);
    assert_eq!(super::super::compute_crc(&frame[1..45], 123), wire_crc);
}
