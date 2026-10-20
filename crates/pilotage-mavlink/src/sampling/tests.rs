use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::codec::{
    ATTITUDE_QUATERNION_ID, AVIATE_ESTIMATOR_STATUS_ID, LOCAL_POSITION_NED_ID, encode_frame_v2_from,
};
use crate::link::apply_messages_at;
use crate::{LinkState, parse_datagram};

use super::operational_pose;

fn receive(state: &Arc<Mutex<LinkState>>, id: u32, payload: &[u8], crc: u8) {
    let mut bytes = vec![0; payload.len() + 12];
    let length = encode_frame_v2_from(0, id, payload, crc, 1, 1, &mut bytes);
    let mut messages = Vec::new();
    let stats = parse_datagram(&bytes[..length], &mut messages);
    assert_eq!(stats.decoded, 1);
    apply_messages_at(state, &messages, 0, 0, Instant::now());
}

fn authorized_pose(quaternion: [f32; 4], position: [f32; 3]) -> Arc<Mutex<LinkState>> {
    let state = Arc::new(Mutex::new(LinkState::default()));
    let mut status = [0_u8; 10];
    status[..8].copy_from_slice(&1_000_000_u64.to_le_bytes());
    status[8] = 15;
    status[9] = 2;
    receive(&state, AVIATE_ESTIMATOR_STATUS_ID, &status, 171);
    let mut attitude = [0_u8; 32];
    attitude[..4].copy_from_slice(&1_000_u32.to_le_bytes());
    for (index, value) in quaternion.into_iter().enumerate() {
        attitude[4 + index * 4..8 + index * 4].copy_from_slice(&value.to_le_bytes());
    }
    receive(&state, ATTITUDE_QUATERNION_ID, &attitude, 246);
    let mut kinematics = [0_u8; 28];
    kinematics[..4].copy_from_slice(&1_000_u32.to_le_bytes());
    for (index, value) in position.into_iter().enumerate() {
        kinematics[4 + index * 4..8 + index * 4].copy_from_slice(&value.to_le_bytes());
    }
    receive(&state, LOCAL_POSITION_NED_ID, &kinematics, 185);
    state
}

#[test]
fn decoded_authorized_unit_quaternions_and_finite_positions_support_control() {
    for quaternion in [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
        [
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
            0.0,
            std::f32::consts::FRAC_1_SQRT_2,
        ],
    ] {
        let state = authorized_pose(quaternion, [1.0, 2.0, -3.0]);
        let latest = state.lock().expect("cache");
        let pose = operational_pose(&latest).expect("valid pose");
        assert_eq!(pose.pos_ned_m, [1.0, 2.0, -3.0]);
        assert!(pose.yaw_rad.is_finite());
    }
}

#[test]
fn decoded_zero_nonunit_and_nonfinite_quaternions_do_not_support_control() {
    for quaternion in [
        [0.0; 4],
        [2.0, 0.0, 0.0, 0.0],
        [0.5, 0.0, 0.0, 0.0],
        [f32::NAN, 0.0, 0.0, 0.0],
        [1.0, f32::INFINITY, 0.0, 0.0],
    ] {
        let state = authorized_pose(quaternion, [0.0; 3]);
        assert!(operational_pose(&state.lock().expect("cache")).is_none());
    }
}

#[test]
fn decoded_nonfinite_position_does_not_support_control_even_with_authorization() {
    for position in [
        [f32::NAN, 0.0, 0.0],
        [0.0, f32::INFINITY, 0.0],
        [0.0, 0.0, f32::NEG_INFINITY],
    ] {
        let state = authorized_pose([1.0, 0.0, 0.0, 0.0], position);
        assert!(operational_pose(&state.lock().expect("cache")).is_none());
    }
}
