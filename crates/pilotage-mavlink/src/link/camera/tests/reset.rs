use super::*;
use crate::codec::{
    ATTITUDE_QUATERNION_ID, CAMERA_INFORMATION_ID, CAMERA_SETTINGS_ID, encode_frame_v2_from,
};
use crate::{CameraWriteRequirement, ResetPolicy, SendFence, parse_datagram};

fn receive(
    state: &Arc<Mutex<LinkState>>,
    message: u32,
    payload: &[u8],
    crc: u8,
    component: u8,
    at: Instant,
) {
    let mut bytes = vec![0; payload.len() + 12];
    let size = encode_frame_v2_from(0, message, payload, crc, 1, component, &mut bytes);
    let mut messages = Vec::new();
    let stats = parse_datagram(&bytes[..size], &mut messages);
    assert_eq!(stats.decoded, 1);
    crate::link::apply_messages_at(state, &messages, 0, 0, at);
}

fn attitude(state: &Arc<Mutex<LinkState>>, time: u32, at: Instant) {
    let mut payload = [0_u8; 32];
    payload[..4].copy_from_slice(&time.to_le_bytes());
    payload[4..8].copy_from_slice(&1.0_f32.to_le_bytes());
    receive(state, ATTITUDE_QUATERNION_ID, &payload, 246, 1, at);
}

fn information(state: &Arc<Mutex<LinkState>>, key: (u8, u8), time: u32, at: Instant) {
    let mut payload = [0_u8; 237];
    payload[..4].copy_from_slice(&time.to_le_bytes());
    payload[20..24].copy_from_slice(&64_u32.to_le_bytes());
    payload[236] = key.1;
    receive(state, CAMERA_INFORMATION_ID, &payload, 92, key.0, at);
}

fn settings(state: &Arc<Mutex<LinkState>>, key: (u8, u8), time: u32, at: Instant) {
    let mut payload = [0_u8; 14];
    payload[..4].copy_from_slice(&time.to_le_bytes());
    payload[5..9].copy_from_slice(&25.0_f32.to_le_bytes());
    payload[13] = key.1;
    receive(state, CAMERA_SETTINGS_ID, &payload, 146, key.0, at);
}

fn requirement(state: &Arc<Mutex<LinkState>>, key: (u8, u8)) -> CameraWriteRequirement {
    let state = state.lock().unwrap();
    let camera = state.fc_reports.cameras[&key];
    CameraWriteRequirement {
        key,
        epoch: camera.epoch,
        revision: camera.capability_revision,
        source_id: state.source_id,
        source_incarnation: state.source_incarnation,
        source_epoch: state.source_epoch,
    }
}

#[test]
fn confirmed_source_reset_requires_new_camera_discovery_and_fences_prior_commands() {
    for key in [(100, 0), (1, 2)] {
        let state = Arc::new(Mutex::new(LinkState {
            reset_policy: ResetPolicy::SimulatorHeuristic,
            ..LinkState::default()
        }));
        let base = Instant::now() - Duration::from_secs(4);
        attitude(&state, 120_000, base);
        information(&state, key, 1_000, base);
        settings(&state, key, 1_001, base);
        let initial = requirement(&state, key);
        let fence = SendFence::default().for_camera(state.clone(), initial);
        assert!(fence.with_valid_write(|| Ok(true)).unwrap());
        attitude(&state, 0, base + Duration::from_secs(3));
        assert_eq!(state.lock().unwrap().source_resets, 0);
        assert_eq!(requirement(&state, key), initial);
        attitude(&state, 250, base + Duration::from_millis(3_250));
        {
            let state = state.lock().unwrap();
            let report = state.fc_reports.cameras[&key];
            assert_eq!(state.source_resets, 1);
            assert_eq!(state.source_epoch, initial.source_epoch.wrapping_add(1));
            assert_eq!(report.epoch, initial.epoch.wrapping_add(1));
            assert_eq!(report.capability_revision, initial.revision.wrapping_add(1));
            assert!(!report.available);
            assert_eq!(report.travel_fraction, None);
            assert_eq!(report.information_received_at, None);
            assert_eq!(report.settings_received_at, None);
            assert_eq!(report.time_boot_ms, 1_001);
        }
        let now = Instant::now();
        information(&state, key, 1_000, now);
        settings(&state, key, 1_001, now);
        {
            let state = state.lock().unwrap();
            let report = state.fc_reports.cameras[&key];
            assert!(!report.available);
            assert_eq!(report.travel_fraction, None);
        }
        settings(&state, key, 1_002, now);
        assert!(!state.lock().unwrap().fc_reports.cameras[&key].available);
        information(&state, key, 1_003, now);
        assert!(state.lock().unwrap().fc_reports.cameras[&key].available);
        assert!(
            !fence
                .with_valid_write(|| panic!("new discovery cannot authorize an old command"))
                .unwrap()
        );
        let renewed = SendFence::default().for_camera(state.clone(), requirement(&state, key));
        assert!(renewed.with_valid_write(|| Ok(true)).unwrap());
    }
}
