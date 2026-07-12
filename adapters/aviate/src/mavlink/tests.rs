#![allow(clippy::expect_used, clippy::panic)]

use super::{
    ATTITUDE_QUATERNION_ID, AviateMessage, FrameSource, LOCAL_POSITION_NED_ID, MAGIC_V2,
    encode_gcs_heartbeat, parse_datagram,
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

    let mut out: Vec<(FrameSource, AviateMessage)> = Vec::new();
    let stats = parse_datagram(&datagram, &mut out);
    assert_eq!(stats.decoded, 2);
    assert_eq!(stats.crc_failures, 0);
    assert_eq!(
        out[0],
        (
            SOURCE,
            AviateMessage::AttitudeQuaternion {
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
            AviateMessage::LocalPositionNed {
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
    let mut out: Vec<(FrameSource, AviateMessage)> = Vec::new();
    let stats = parse_datagram(&frame, &mut out);
    assert_eq!(stats.decoded, 1, "stats: {stats:?}");
    assert_eq!(
        out[0],
        (
            SOURCE,
            AviateMessage::AttitudeQuaternion {
                time_boot_ms: 99,
                quat_wxyz: q,
                rates_rps: [0.0; 3],
            }
        )
    );
}

#[test]
fn corrupted_frame_fails_crc_and_is_skipped() {
    let mut frame = encode_frame(
        LOCAL_POSITION_NED_ID,
        &local_position_payload([1.0; 3], [0.5; 3], 7),
        false,
    );
    frame[12] ^= 0xff;
    let mut out: Vec<(FrameSource, AviateMessage)> = Vec::new();
    let stats = parse_datagram(&frame, &mut out);
    assert_eq!(stats.crc_failures, 1);
    assert!(out.is_empty());
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
    let mut out: Vec<(FrameSource, AviateMessage)> = Vec::new();
    let stats = parse_datagram(&datagram, &mut out);
    assert_eq!(stats.unknown_ids, 1);
    assert_eq!(stats.decoded, 1, "the frame after the unknown id decodes");
}

#[test]
fn garbage_prefix_resyncs_to_the_next_frame() {
    let mut datagram = vec![0x00, 0x42, 0x13];
    datagram.extend_from_slice(&encode_frame(0, &[0u8; 9], false));
    let mut out: Vec<(FrameSource, AviateMessage)> = Vec::new();
    let stats = parse_datagram(&datagram, &mut out);
    assert_eq!(stats.garbage_bytes, 3);
    assert_eq!(
        out,
        vec![(SOURCE, AviateMessage::Heartbeat { armed: false })]
    );
}

#[test]
fn gcs_heartbeat_parses_back_as_heartbeat() {
    let frame = encode_gcs_heartbeat(4);
    let mut out: Vec<(FrameSource, AviateMessage)> = Vec::new();
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
            AviateMessage::Heartbeat { armed: false }
        )]
    );
}

#[test]
fn truncated_tail_is_garbage_not_panic() {
    let frame = encode_frame(0, &[0u8; 9], false);
    let mut out: Vec<(FrameSource, AviateMessage)> = Vec::new();
    let stats = parse_datagram(&frame[..frame.len() - 3], &mut out);
    assert!(out.is_empty());
    assert!(stats.garbage_bytes > 0);
}
