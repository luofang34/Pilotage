#![allow(clippy::expect_used, clippy::panic)]

use super::{
    ATTITUDE_QUATERNION_ID, AVIATE_ESTIMATOR_STATUS_ID, ESTIMATOR_STATUS_ID, FcMessage,
    FrameSource, LOCAL_POSITION_NED_ID, MAGIC_V2, encode_gcs_heartbeat, parse_datagram,
};

const SOURCE: FrameSource = FrameSource {
    system_id: 1,
    component_id: 1,
    frame_sequence: 7,
};

/// Test-side serializer: builds a MAVLink 2.0 frame the way Aviate's
/// `aviate-link` does, optionally truncating trailing zero payload bytes
/// (the v2 wire optimization the parser must zero-extend back).
fn encode_frame(msg_id: u32, payload: &[u8], truncate: bool) -> Vec<u8> {
    let mut p = payload.to_vec();
    if truncate {
        while p.len() > 1 && p.last() == Some(&0) {
            p.pop();
        }
    }
    let mut frame = vec![
        MAGIC_V2,
        p.len() as u8,
        0,
        0,
        7, // seq
        1, // sysid
        1, // compid
        (msg_id & 0xff) as u8,
        ((msg_id >> 8) & 0xff) as u8,
        ((msg_id >> 16) & 0xff) as u8,
    ];
    frame.extend_from_slice(&p);
    let extra = match msg_id {
        0 => 50,
        31 => 246,
        32 => 185,
        230 => 163,
        285 => 137,
        20_000 => 171,
        other => (other & 0xff) as u8,
    };
    let crc = super::compute_crc(&frame[1..], extra);
    frame.push((crc & 0xff) as u8);
    frame.push((crc >> 8) as u8);
    frame
}

fn attitude_payload(q: [f32; 4], rates: [f32; 3], t: u32) -> Vec<u8> {
    let mut p = t.to_le_bytes().to_vec();
    for v in q.iter().chain(rates.iter()) {
        p.extend_from_slice(&v.to_le_bytes());
    }
    // MAVLink2 extension repr_offset_q[4] as zeros (full 48-byte layout).
    p.extend_from_slice(&[0u8; 16]);
    p
}

fn local_position_payload(pos: [f32; 3], vel: [f32; 3], t: u32) -> Vec<u8> {
    let mut p = t.to_le_bytes().to_vec();
    for v in pos.iter().chain(vel.iter()) {
        p.extend_from_slice(&v.to_le_bytes());
    }
    p
}

#[test]
fn decodes_attitude_and_position_from_one_datagram() {
    let q = [0.9f32, 0.1, 0.2, 0.3];
    let rates = [0.01f32, 0.02, 0.03];
    let pos = [1.0f32, 2.0, -30.0];
    let vel = [4.0f32, 0.0, -1.5];
    let mut datagram = encode_frame(
        ATTITUDE_QUATERNION_ID,
        &attitude_payload(q, rates, 1234),
        false,
    );
    datagram.extend_from_slice(&encode_frame(
        LOCAL_POSITION_NED_ID,
        &local_position_payload(pos, vel, 1250),
        false,
    ));

    let mut out: Vec<(FrameSource, FcMessage)> = Vec::new();
    let stats = parse_datagram(&datagram, &mut out);
    assert_eq!(stats.decoded, 2);
    assert_eq!(stats.crc_failures, 0);
    assert_eq!(
        out[0],
        (
            SOURCE,
            FcMessage::AttitudeQuaternion {
                time_boot_ms: 1234,
                quat_wxyz: q,
                rates_rps: rates,
            }
        )
    );
    assert_eq!(
        out[1],
        (
            SOURCE,
            FcMessage::LocalPositionNed {
                time_boot_ms: 1250,
                pos_ned_m: pos,
                vel_ned_mps: vel,
            }
        )
    );
}

#[test]
fn v2_truncated_payload_zero_extends() {
    // Attitude with zero rates: trailing zeros truncate on the wire.
    let q = [1.0f32, 0.0, 0.0, 0.0];
    let frame = encode_frame(
        ATTITUDE_QUATERNION_ID,
        &attitude_payload(q, [0.0; 3], 99),
        true,
    );
    let mut out: Vec<(FrameSource, FcMessage)> = Vec::new();
    let stats = parse_datagram(&frame, &mut out);
    assert_eq!(stats.decoded, 1, "stats: {stats:?}");
    assert_eq!(
        out[0],
        (
            SOURCE,
            FcMessage::AttitudeQuaternion {
                time_boot_ms: 99,
                quat_wxyz: q,
                rates_rps: [0.0; 3],
            }
        )
    );
}

#[test]
fn decodes_aviate_estimator_status_golden_vectors() {
    let cases = [
        (
            vec![
                253, 10, 0, 0, 7, 1, 1, 32, 78, 0, 184, 130, 1, 0, 0, 0, 0, 0, 15, 2, 238, 73,
            ],
            FcMessage::AviateEstimatorStatus {
                time_usec: 99_000,
                valid_flags: 0x0f,
                quality: 2,
            },
            7,
        ),
        (
            vec![
                253, 10, 0, 0, 4, 1, 1, 32, 78, 0, 144, 208, 3, 0, 0, 0, 0, 0, 3, 1, 137, 232,
            ],
            FcMessage::AviateEstimatorStatus {
                time_usec: 250_000,
                valid_flags: 0x03,
                quality: 1,
            },
            4,
        ),
        (
            vec![253, 2, 0, 0, 7, 1, 1, 32, 78, 0, 104, 66, 104, 226],
            FcMessage::AviateEstimatorStatus {
                time_usec: 17_000,
                valid_flags: 0,
                quality: 0,
            },
            7,
        ),
    ];

    for (frame, expected, frame_sequence) in cases {
        let mut out = Vec::new();
        let stats = parse_datagram(&frame, &mut out);
        assert_eq!(stats.decoded, 1, "stats: {stats:?}");
        assert_eq!(
            out,
            vec![(
                FrameSource {
                    frame_sequence,
                    ..SOURCE
                },
                expected
            )]
        );
    }
}

#[test]
fn standard_estimator_status_is_known_but_distinct() {
    let frame = [253, 2, 0, 0, 7, 1, 1, 230, 0, 0, 104, 66, 51, 209];
    let mut out = Vec::new();
    let stats = parse_datagram(&frame, &mut out);
    assert_eq!(stats.decoded, 1, "stats: {stats:?}");
    assert_eq!(
        out,
        vec![(
            SOURCE,
            FcMessage::EstimatorStatus {
                time_usec: 17_000,
                flags: 0,
            }
        )]
    );
}

#[test]
fn status_test_serializer_uses_the_canonical_crc_extras() {
    let private = encode_frame(AVIATE_ESTIMATOR_STATUS_ID, &[0; 10], false);
    let standard = encode_frame(ESTIMATOR_STATUS_ID, &[0; 42], false);
    let mut out = Vec::new();
    let stats = parse_datagram(&private, &mut out);
    assert_eq!(stats.decoded, 1);
    let stats = parse_datagram(&standard, &mut out);
    assert_eq!(stats.decoded, 1);
}

#[test]
fn corrupted_frame_fails_crc_and_is_skipped() {
    let mut frame = encode_frame(
        LOCAL_POSITION_NED_ID,
        &local_position_payload([1.0; 3], [0.5; 3], 7),
        false,
    );
    frame[12] ^= 0xff;
    let mut out: Vec<(FrameSource, FcMessage)> = Vec::new();
    let stats = parse_datagram(&frame, &mut out);
    assert_eq!(stats.crc_failures, 1);
    assert_eq!(stats.invalid_estimator_status_frames, 0);
    assert!(out.is_empty());
}

#[test]
fn corrupted_private_status_is_an_explicit_revocation_signal() {
    let mut frame = encode_frame(AVIATE_ESTIMATOR_STATUS_ID, &[0; 10], false);
    frame[10] ^= 0xff;
    let mut out = Vec::new();
    let stats = parse_datagram(&frame, &mut out);
    assert_eq!(stats.crc_failures, 1);
    assert_eq!(stats.invalid_estimator_status_frames, 1);
    assert_eq!(out, vec![(SOURCE, FcMessage::InvalidAviateEstimatorStatus)]);
}

#[test]
fn truncated_private_status_is_an_explicit_revocation_signal() {
    let frame = encode_frame(AVIATE_ESTIMATOR_STATUS_ID, &[0; 10], false);
    let mut out = Vec::new();
    let stats = parse_datagram(&frame[..frame.len() - 1], &mut out);
    assert_eq!(stats.invalid_estimator_status_frames, 1);
    assert!(stats.garbage_bytes > 0);
    assert_eq!(out, vec![(SOURCE, FcMessage::InvalidAviateEstimatorStatus)]);
}

#[test]
fn unsupported_incompatibility_flags_never_authorize_private_status() {
    let mut frame = encode_frame(AVIATE_ESTIMATOR_STATUS_ID, &[0; 10], false);
    frame[2] = 0x02;
    let mut out = Vec::new();
    let stats = parse_datagram(&frame, &mut out);
    assert_eq!(stats.invalid_estimator_status_frames, 1);
    assert!(stats.garbage_bytes > 0);
    assert_eq!(out, vec![(SOURCE, FcMessage::InvalidAviateEstimatorStatus)]);
}

#[test]
fn invalid_private_status_event_preserves_datagram_order() {
    let mut datagram = encode_frame(AVIATE_ESTIMATOR_STATUS_ID, &[0; 10], false);
    let mut invalid = encode_frame(AVIATE_ESTIMATOR_STATUS_ID, &[0; 10], false);
    invalid[10] ^= 0xff;
    datagram.extend_from_slice(&invalid);
    let mut out = Vec::new();
    let stats = parse_datagram(&datagram, &mut out);
    assert_eq!(stats.decoded, 1);
    assert_eq!(stats.invalid_estimator_status_frames, 1);
    assert!(matches!(
        out.as_slice(),
        [
            (_, FcMessage::AviateEstimatorStatus { .. }),
            (_, FcMessage::InvalidAviateEstimatorStatus)
        ]
    ));
}

#[test]
fn unknown_message_id_is_counted_and_skipped_whole() {
    let unknown = encode_frame(99, &[1, 2, 3, 4], false);
    let mut datagram = unknown;
    datagram.extend_from_slice(&encode_frame(
        LOCAL_POSITION_NED_ID,
        &local_position_payload([0.0; 3], [1.0; 3], 1),
        false,
    ));
    let mut out: Vec<(FrameSource, FcMessage)> = Vec::new();
    let stats = parse_datagram(&datagram, &mut out);
    assert_eq!(stats.unknown_ids, 1);
    assert_eq!(stats.decoded, 1, "the frame after the unknown id decodes");
}

#[test]
fn garbage_prefix_resyncs_to_the_next_frame() {
    let mut datagram = vec![0x00, 0x42, 0x13];
    datagram.extend_from_slice(&encode_frame(0, &[0u8; 9], false));
    let mut out: Vec<(FrameSource, FcMessage)> = Vec::new();
    let stats = parse_datagram(&datagram, &mut out);
    assert_eq!(stats.garbage_bytes, 3);
    assert_eq!(out, vec![(SOURCE, FcMessage::Heartbeat { armed: false })]);
}

#[test]
fn gcs_heartbeat_parses_back_as_heartbeat() {
    let frame = encode_gcs_heartbeat(4);
    let mut out: Vec<(FrameSource, FcMessage)> = Vec::new();
    let stats = parse_datagram(&frame, &mut out);
    assert_eq!(stats.decoded, 1, "stats: {stats:?}");
    assert_eq!(
        out,
        vec![(
            FrameSource {
                system_id: 255,
                component_id: 190,
                frame_sequence: 4,
            },
            FcMessage::Heartbeat { armed: false }
        )]
    );
}

#[test]
fn truncated_tail_is_garbage_not_panic() {
    let frame = encode_frame(0, &[0u8; 9], false);
    let mut out: Vec<(FrameSource, FcMessage)> = Vec::new();
    let stats = parse_datagram(&frame[..frame.len() - 3], &mut out);
    assert!(out.is_empty());
    assert!(stats.garbage_bytes > 0);
}

/// Full 49-byte GIMBAL_DEVICE_ATTITUDE_STATUS payload: base fields
/// through the targets, then the v2 extension tail the decoder ignores.
fn gimbal_status_payload(
    q: [f32; 4],
    rates: [f32; 3],
    t: u32,
    failure: u32,
    flags: u16,
) -> Vec<u8> {
    let mut p = t.to_le_bytes().to_vec();
    for v in q.iter().chain(rates.iter()) {
        p.extend_from_slice(&v.to_le_bytes());
    }
    p.extend_from_slice(&failure.to_le_bytes());
    p.extend_from_slice(&flags.to_le_bytes());
    p.push(255); // target_system
    p.push(190); // target_component
    p.extend_from_slice(&0.25f32.to_le_bytes()); // ext: delta_yaw
    p.extend_from_slice(&0.0f32.to_le_bytes()); // ext: delta_yaw_velocity
    p.push(0); // ext: gimbal_device_id
    p
}

#[test]
fn decodes_gimbal_device_attitude_status() {
    let q = [0.98f32, 0.0, -0.19, 0.0];
    let rates = [0.0f32, 0.05, -0.02];
    let frame = encode_frame(
        super::GIMBAL_DEVICE_ATTITUDE_STATUS_ID,
        &gimbal_status_payload(q, rates, 5_000, 0, 12),
        false,
    );
    let mut out: Vec<(FrameSource, FcMessage)> = Vec::new();
    let stats = parse_datagram(&frame, &mut out);
    assert_eq!(stats.decoded, 1, "stats: {stats:?}");
    assert_eq!(
        out,
        vec![(
            SOURCE,
            FcMessage::GimbalDeviceAttitudeStatus {
                time_boot_ms: 5_000,
                quat_wxyz: q,
                rates_rps: rates,
                flags: 12,
                failure_flags: 0,
            }
        )]
    );
}

#[test]
fn gimbal_status_v2_truncated_payload_zero_extends() {
    // Zero failure flags and zero extension tail truncate on the wire;
    // the decoder must recover flags at their full-layout offset.
    let q = [1.0f32, 0.0, 0.0, 0.0];
    let frame = encode_frame(
        super::GIMBAL_DEVICE_ATTITUDE_STATUS_ID,
        &gimbal_status_payload(q, [0.0; 3], 7, 0, 12),
        true,
    );
    let mut out: Vec<(FrameSource, FcMessage)> = Vec::new();
    let stats = parse_datagram(&frame, &mut out);
    assert_eq!(stats.decoded, 1, "stats: {stats:?}");
    match out[0].1 {
        FcMessage::GimbalDeviceAttitudeStatus {
            time_boot_ms,
            quat_wxyz,
            flags,
            failure_flags,
            ..
        } => {
            assert_eq!(time_boot_ms, 7);
            assert_eq!(quat_wxyz, q);
            assert_eq!(flags, 12);
            assert_eq!(failure_flags, 0);
        }
        other => panic!("unexpected message: {other:?}"),
    }
}

#[test]
fn gimbal_rate_setpoint_locks_the_wire_layout() {
    let frame = super::encode_gimbal_rate_setpoint(9, 0.4, -0.6, 1, 1);
    // Header: v2 magic, 35-byte payload, GCS identity, msg id 282.
    assert_eq!(frame[0], MAGIC_V2);
    assert_eq!(frame[1], 35);
    assert_eq!(frame[4], 9);
    assert_eq!(frame[5], 255);
    assert_eq!(frame[6], 190);
    assert_eq!(
        u32::from(frame[7]) | (u32::from(frame[8]) << 8) | (u32::from(frame[9]) << 16),
        super::GIMBAL_MANAGER_SET_ATTITUDE_ID
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
    assert_eq!(super::compute_crc(&frame[1..45], 123), wire_crc);
}
