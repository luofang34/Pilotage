use super::*;
use pilotage_mavlink::codec::{
    ATTITUDE_QUATERNION_ID, CAMERA_INFORMATION_ID, CAMERA_SETTINGS_ID, encode_frame_v2_from,
};

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
    let stats = pilotage_mavlink::parse_datagram(&bytes[..size], &mut messages);
    assert_eq!(stats.decoded, 1);
    pilotage_mavlink::link::apply_messages_at(state, &messages, 0, 0, at);
}

fn attitude(state: &Arc<Mutex<LinkState>>, time: u32, at: Instant) {
    let mut payload = [0_u8; 32];
    payload[..4].copy_from_slice(&time.to_le_bytes());
    payload[4..8].copy_from_slice(&1.0_f32.to_le_bytes());
    receive(state, ATTITUDE_QUATERNION_ID, &payload, 246, 1, at);
}

fn rediscover(state: &Arc<Mutex<LinkState>>, at: Instant) {
    let mut info = [0_u8; 237];
    info[..4].copy_from_slice(&102_u32.to_le_bytes());
    info[20..24].copy_from_slice(&64_u32.to_le_bytes());
    receive(state, CAMERA_INFORMATION_ID, &info, 92, 100, at);
    let mut settings = [0_u8; 14];
    settings[..4].copy_from_slice(&103_u32.to_le_bytes());
    settings[5..9].copy_from_slice(&25.0_f32.to_le_bytes());
    receive(state, CAMERA_SETTINGS_ID, &settings, 146, 100, at);
}

#[test]
fn confirmed_source_reset_fences_zoom_and_progress_before_adapter_synchronizes() {
    runtime().block_on(async {
        let (link, receiver) = transport().await;
        let state = link.state();
        state.lock().unwrap().reset_policy = pilotage_mavlink::ResetPolicy::SimulatorHeuristic;
        let now = Instant::now();
        attitude(&state, 120_000, now - Duration::from_secs(4));
        let (staging, mut queued) = tokio::sync::mpsc::channel(8);
        let mut adapter = Px4Adapter::from_state(VehicleId::new(1), state.clone());
        adapter.camera_control = Some(CameraControl::new(staging, 1));
        report(&state, now);
        let command = frame(ZoomCommand::Step(1));
        assert!(adapter.apply_control(&command).action_results[0].accepted);
        let old_command = queued.try_recv().unwrap();
        let old_requirement = requirement(&state);
        attitude(&state, 0, now - Duration::from_millis(300));
        attitude(&state, 250, now);
        assert_eq!(
            state.lock().unwrap().source_epoch,
            old_requirement.source_epoch.wrapping_add(1)
        );
        let mut batch = TelemetryBatch::default();
        adapter.append_camera_samples(&mut batch);
        assert!(
            batch.samples.is_empty(),
            "old progress cannot acquire the reset identity"
        );
        rediscover(&state, Instant::now());
        adapter.append_camera_samples(&mut batch);
        let payload = batch.samples[0].payload.as_ref().unwrap();
        assert_eq!(payload.zoom.as_ref().unwrap().value, 0.25);
        assert!(
            payload.command.is_none(),
            "new discovery cannot relabel old progress"
        );
        link.command_sender().send(old_command).await.unwrap();
        sentinel(&link).await;
        assert_eq!(receive_command(&receiver).await.0, 512);
        let control = adapter.camera_control.as_ref().unwrap();
        assert!(
            control
                .progress_cached(old_requirement)
                .unwrap()
                .sent_at
                .is_none()
        );
        assert!(control.progress_cached(requirement(&state)).is_none());
        adapter.maintain_camera();
        assert!(
            adapter
                .camera_control
                .as_ref()
                .unwrap()
                .progress_cached(requirement(&state))
                .is_none()
        );
    });
}
