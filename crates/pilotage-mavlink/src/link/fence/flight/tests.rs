use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pilotage_adapter_api::SourceIncarnation;

use crate::codec::{
    ATTITUDE_QUATERNION_ID, EKF_STATUS_REPORT_ID, EXTENDED_SYS_STATE_ID, GLOBAL_POSITION_INT_ID,
    HEARTBEAT_ID, LOCAL_POSITION_NED_ID, encode_frame_v2_from,
};
use crate::link::apply_messages_at;
use crate::{AuthorizationSource, LinkState, SendFence, TakeoffWriteRequirement, parse_datagram};

fn receive(
    state: &Arc<Mutex<LinkState>>,
    message_id: u32,
    payload: &[u8],
    crc: u8,
    source: (u8, u8),
    received: Instant,
) {
    let mut bytes = vec![0; payload.len() + 12];
    let size = encode_frame_v2_from(0, message_id, payload, crc, source.0, source.1, &mut bytes);
    let mut messages = Vec::new();
    let stats = parse_datagram(&bytes[..size], &mut messages);
    assert_eq!(stats.decoded, 1);
    apply_messages_at(state, &messages, 0, 0, received);
}

fn heartbeat(state: &Arc<Mutex<LinkState>>, armed: bool, mode: u8, at: Instant) {
    receive(
        state,
        HEARTBEAT_ID,
        &[mode, 0, 0, 0, 2, 3, if armed { 0x80 } else { 0 }, 4, 3],
        50,
        (23, 1),
        at,
    );
}

fn numeric(state: &Arc<Mutex<LinkState>>, time_ms: u32, down: f32, received: Instant) {
    let mut attitude = [0_u8; 32];
    attitude[..4].copy_from_slice(&time_ms.to_le_bytes());
    attitude[4..8].copy_from_slice(&1.0_f32.to_le_bytes());
    receive(
        state,
        ATTITUDE_QUATERNION_ID,
        &attitude,
        246,
        (23, 1),
        received,
    );
    let mut position = [0_u8; 28];
    position[..4].copy_from_slice(&time_ms.to_le_bytes());
    position[12..16].copy_from_slice(&down.to_le_bytes());
    receive(
        state,
        LOCAL_POSITION_NED_ID,
        &position,
        185,
        (23, 1),
        received,
    );
}

fn ready(received: Instant) -> Arc<Mutex<LinkState>> {
    let state = Arc::new(Mutex::new(LinkState {
        system_id: 23,
        component_id: 1,
        source_id: 19,
        source_incarnation: SourceIncarnation::new([8; 16]),
        started_at: received,
        maximum_inter_group_skew_ms: 300,
        authorization_source: AuthorizationSource::ArduPilotEkfStatus,
        ..LinkState::default()
    }));
    numeric(&state, 1_000, 0.0, received);
    let mut status = [0_u8; 22];
    status[20..22].copy_from_slice(&63_u16.to_le_bytes());
    receive(&state, EKF_STATUS_REPORT_ID, &status, 71, (23, 1), received);
    numeric(&state, 1_100, 0.0, received);
    receive(
        &state,
        GLOBAL_POSITION_INT_ID,
        &1_100_u32.to_le_bytes(),
        104,
        (23, 1),
        received,
    );
    receive(
        &state,
        EXTENDED_SYS_STATE_ID,
        &[0, 1],
        130,
        (23, 1),
        received,
    );
    heartbeat(&state, true, 4, received);
    state
}

fn fence(state: &Arc<Mutex<LinkState>>) -> SendFence {
    let requirement =
        TakeoffWriteRequirement::new(&state.lock().expect("cache"), 4, Duration::from_secs(3));
    SendFence::default().for_takeoff(state.clone(), requirement)
}

fn blocked(fence: &SendFence) {
    assert!(
        !fence
            .with_valid_write(|| panic!("invalidated launch preconditions must not write"))
            .expect("fence")
    );
}

#[test]
fn selected_flight_reports_remain_locked_through_the_actual_write() {
    let state = ready(Instant::now());
    assert!(
        fence(&state)
            .with_valid_write(|| {
                assert!(
                    state.try_lock().is_err(),
                    "receive updates cannot race the write"
                );
                Ok(true)
            })
            .expect("guarded write")
    );
}

#[test]
fn disarm_or_mode_change_before_adapter_poll_blocks_queued_takeoff() {
    for (armed, mode) in [(false, 4), (true, 5)] {
        let now = Instant::now();
        let state = ready(now);
        let fence = fence(&state);
        heartbeat(&state, armed, mode, now);
        blocked(&fence);
    }
}

#[test]
fn non_ground_landed_reports_block_queued_takeoff() {
    for landed in [0, 2, 3, 4, 255] {
        let now = Instant::now();
        let state = ready(now);
        let fence = fence(&state);
        receive(
            &state,
            EXTENDED_SYS_STATE_ID,
            &[0, landed],
            130,
            (23, 1),
            now,
        );
        blocked(&fence);
    }
}

#[test]
fn stale_receive_evidence_cannot_authorize_a_final_write() {
    let state = ready(Instant::now() - Duration::from_secs(4));
    blocked(&fence(&state));
}

#[test]
fn invalid_numeric_pose_blocks_a_final_write() {
    let now = Instant::now();
    let state = ready(now);
    let fence = fence(&state);
    numeric(&state, 1_200, f32::NAN, now);
    blocked(&fence);
}

#[test]
fn a_home_reference_jump_cannot_send_the_admitted_takeoff_target() {
    let now = Instant::now();
    let state = ready(now);
    let fence = fence(&state);
    let mut altitude = [0_u8; 20];
    altitude[..4].copy_from_slice(&1_200_u32.to_le_bytes());
    altitude[16..20].copy_from_slice(&3_000_i32.to_le_bytes());
    receive(&state, GLOBAL_POSITION_INT_ID, &altitude, 104, (23, 1), now);
    blocked(&fence);
}

#[test]
fn a_source_identity_change_cannot_reuse_the_admission_snapshot() {
    let now = Instant::now();
    let state = ready(now);
    let initial = state.lock().expect("cache");
    let requirement = TakeoffWriteRequirement::new(&initial, 4, Duration::from_secs(3));
    drop(initial);
    let replacement = ready(now);
    replacement.lock().expect("cache").source_incarnation = SourceIncarnation::new([9; 16]);
    blocked(&SendFence::default().for_takeoff(replacement, requirement));
}

#[test]
fn a_confirmed_clock_reset_blocks_the_queued_source_epoch() {
    let now = Instant::now();
    let state = ready(now);
    let fence = fence(&state);
    {
        let mut latest = state.lock().expect("cache");
        latest.reset_policy = crate::ResetPolicy::SimulatorHeuristic;
    }
    let at = now + Duration::from_secs(3);
    numeric(&state, 100, 0.0, at);
    numeric(&state, 350, 0.0, at + Duration::from_millis(250));
    blocked(&fence);
    assert_eq!(state.lock().expect("cache").source_epoch, 2);
}

#[test]
fn another_component_cannot_change_selected_flight_preconditions() {
    let now = Instant::now();
    let state = ready(now);
    let fence = fence(&state);
    receive(&state, EXTENDED_SYS_STATE_ID, &[0, 2], 130, (23, 191), now);
    assert!(
        fence
            .with_valid_write(|| Ok(true))
            .expect("selected FC permits")
    );
    assert_eq!(state.lock().expect("cache").wrong_sources, 1);
}
