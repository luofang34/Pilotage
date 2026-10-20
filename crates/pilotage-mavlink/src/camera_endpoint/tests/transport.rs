use super::*;
use crate::{CommandTarget, LinkConfig, MavlinkLink, OutboundCommand};
use tokio::net::UdpSocket;

async fn transport() -> (MavlinkLink, UdpSocket) {
    let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let config = LinkConfig {
        endpoint: "127.0.0.1:0".parse().unwrap(),
        command_target: CommandTarget::Fixed(receiver.local_addr().unwrap()),
        ..LinkConfig::default()
    };
    let link = MavlinkLink::start(config, SourceIncarnation::new([5; 16]))
        .await
        .unwrap();
    (link, receiver)
}

async fn receive_command(receiver: &UdpSocket) -> (u16, Vec<u8>) {
    let mut bytes = [0; 300];
    let length = tokio::time::timeout(Duration::from_secs(1), receiver.recv(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&bytes[7..10], &[76, 0, 0]);
    (
        u16::from_le_bytes([bytes[38], bytes[39]]),
        bytes[..length].to_vec(),
    )
}

async fn sentinel(link: &MavlinkLink) {
    link.command_sender()
        .send(OutboundCommand {
            command: 512,
            params: [260.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            target_system: 1,
            target_component: 100,
            send_before: None,
            fence: None,
            payload_write: None,
            receipt: None,
        })
        .await
        .unwrap();
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn actual_camera_write_retains_sent_evidence_without_fabricating_ack_or_feedback() {
    runtime().block_on(async {
        let (link, receiver) = transport().await;
        let state = link.state();
        let mut endpoint = CameraEndpoints::new(
            VehicleId::new(1),
            state.clone(),
            link.command_sender(),
            1,
            Instant::now(),
        );
        report(&state, Instant::now());
        let command = frame(ZoomCommand::Absolute {
            unit: ZoomUnit::TravelFraction,
            value_micros: 750_000,
        });
        endpoint.apply_control(&command, SimTick::new(0));
        let (id, bytes) = receive_command(&receiver).await;
        assert_eq!(id, 531);
        assert_eq!(f32::from_le_bytes(bytes[14..18].try_into().unwrap()), 75.0);
        let progress = progress_cached(&endpoint, (100, 0)).unwrap();
        assert_eq!(progress.stage, wire::PayloadCommandStage::Sent as i32);
        assert!(progress.sent_at.is_some());
        assert!(progress.acknowledged_at.is_none());
        assert!(progress.observed_complete_at.is_none());
        let sampled = sample(&mut endpoint);
        let payload = sampled
            .samples
            .iter()
            .find_map(|sample| sample.payload.as_ref())
            .unwrap();
        assert_eq!(payload.zoom.as_ref().unwrap().value, 0.25);
        assert_eq!(payload.zoom.as_ref().unwrap().sampled_at, None);
        assert_eq!(
            payload
                .command
                .as_ref()
                .unwrap()
                .zoom_target
                .as_ref()
                .unwrap()
                .value_micros,
            750_000
        );
        setting(&state, 102, 0.75);
        let sampled = sample(&mut endpoint);
        let payload = sampled
            .samples
            .iter()
            .find_map(|s| s.payload.as_ref())
            .unwrap();
        assert_eq!(payload.zoom.as_ref().unwrap().value, 0.75);
        assert_eq!(payload.zoom.as_ref().unwrap().sampled_at, None);
        assert!(
            payload
                .command
                .as_ref()
                .unwrap()
                .observed_complete_at
                .is_none()
        );
        assert!(payload.command.as_ref().unwrap().acknowledged_at.is_none());
    });
}

async fn assert_queued_command_fenced(change_capability: bool) {
    let (link, receiver) = transport().await;
    let state = link.state();
    let (staging, mut queued) = tokio::sync::mpsc::channel(8);
    let mut endpoint =
        CameraEndpoints::new(VehicleId::new(1), state.clone(), staging, 1, Instant::now());
    report(&state, Instant::now());
    let command = frame(ZoomCommand::Step(1));
    endpoint.apply_control(&command, SimTick::new(0));
    let stale = queued.try_recv().unwrap();
    if change_capability {
        crate::link::apply_messages_at(
            &state,
            &[(
                FrameSource {
                    system_id: 1,
                    component_id: 100,
                    frame_sequence: 1,
                },
                FcMessage::Camera(CameraMessage::Information {
                    time_boot_ms: 102,
                    flags: 0,
                    device_id: 0,
                    gimbal_id: 0,
                }),
            )],
            0,
            0,
            Instant::now(),
        );
    } else {
        endpoint.invalidate_pending_commands(VehicleId::new(1), &command.scope);
    }
    link.command_sender().send(stale).await.unwrap();
    sentinel(&link).await;
    assert_eq!(
        receive_command(&receiver).await.0,
        512,
        "the ordered sentinel must arrive without a stale zoom before it"
    );
    endpoint.maintain();
    let progress = progress_cached(&endpoint, (100, 0)).unwrap();
    assert!(progress.sent_at.is_none());
    assert_eq!(progress.end_reason, wire::PayloadEndReason::Revoked as i32);
}

#[test]
fn revoked_queued_zoom_cannot_reach_actual_transport() {
    runtime().block_on(assert_queued_command_fenced(false));
}

#[test]
fn raw_camera_capability_change_fences_zoom_before_endpoint_synchronizes() {
    runtime().block_on(assert_queued_command_fenced(true));
}

#[test]
fn attached_cameras_keep_device_routes_observations_and_revocation_separate() {
    runtime().block_on(async {
        let (link, receiver) = transport().await;
        let state = link.state();
        let (staging, mut queued) = tokio::sync::mpsc::channel(8);
        let mut endpoint =
            CameraEndpoints::new(VehicleId::new(1), state.clone(), staging, 1, Instant::now());
        report_camera(&state, (1, 2), 0.2, Instant::now());
        report_camera(&state, (1, 3), 0.8, Instant::now());
        let scopes = endpoint.scopes();
        assert_eq!(scopes.len(), 2);
        let mut first = frame(ZoomCommand::Step(1));
        first.scope = ScopeId::new("vehicle.camera.1.2");
        let mut second = frame(ZoomCommand::Step(-1));
        second.scope = ScopeId::new("vehicle.camera.1.3");
        assert!(
            endpoint
                .apply_control(&first, SimTick::new(0))
                .action_results[0]
                .accepted
        );
        assert!(
            endpoint
                .apply_control(&second, SimTick::new(0))
                .action_results[0]
                .accepted
        );
        endpoint.invalidate_pending_commands(VehicleId::new(99), &second.scope);
        endpoint.invalidate_pending_commands(VehicleId::new(1), &first.scope);
        for expected_device in [2.0, 3.0] {
            let command = queued.try_recv().unwrap();
            assert_eq!(command.target_component, 1);
            assert_eq!(command.params[2], expected_device);
            link.command_sender().send(command).await.unwrap();
        }
        sentinel(&link).await;
        let (id, bytes) = receive_command(&receiver).await;
        assert_eq!(id, 531);
        assert_eq!(bytes[41], 1);
        assert_eq!(f32::from_le_bytes(bytes[18..22].try_into().unwrap()), 3.0);
        assert_eq!(receive_command(&receiver).await.0, 512);
        let batch = sample(&mut endpoint);
        let payloads: Vec<_> = batch
            .samples
            .iter()
            .filter_map(|s| s.payload.as_ref())
            .collect();
        assert_eq!(payloads.len(), 2);
        assert_eq!(payloads[0].endpoint_id, first.scope.as_str());
        assert_eq!(payloads[0].zoom.as_ref().unwrap().value, f64::from(0.2_f32));
        assert!(payloads[0].command.as_ref().unwrap().sent_at.is_none());
        assert_eq!(payloads[1].endpoint_id, second.scope.as_str());
        assert_eq!(payloads[1].zoom.as_ref().unwrap().value, f64::from(0.8_f32));
        assert!(payloads[1].command.as_ref().unwrap().sent_at.is_some());
    });
}

fn attitude(state: &Arc<Mutex<LinkState>>, time: u32, received: Instant) {
    let mut payload = [0; 32];
    payload[..4].copy_from_slice(&time.to_le_bytes());
    payload[4..8].copy_from_slice(&1.0_f32.to_le_bytes());
    let mut bytes = [0; 300];
    let size = crate::codec::encode_frame_v2_from(0, 31, &payload, 246, 1, 1, &mut bytes);
    let mut messages = Vec::new();
    let stats = crate::parse_datagram(&bytes[..size], &mut messages);
    assert_eq!(messages.len(), 1);
    crate::link::apply_messages_at(
        state,
        &messages,
        stats.crc_failures,
        stats.unknown_ids,
        received,
    );
}

#[test]
fn confirmed_source_reset_fences_zoom_before_endpoint_synchronizes() {
    runtime().block_on(async {
        let (link, receiver) = transport().await;
        let state = link.state();
        state.lock().unwrap().reset_policy = crate::ResetPolicy::SimulatorHeuristic;
        let now = Instant::now();
        attitude(&state, 120_000, now - Duration::from_secs(4));
        let (staging, mut queued) = tokio::sync::mpsc::channel(8);
        let mut endpoint = CameraEndpoints::new(VehicleId::new(1), state.clone(), staging, 1, now);
        report(&state, now);
        assert!(
            endpoint
                .apply_control(&frame(ZoomCommand::Step(1)), SimTick::new(0))
                .action_results[0]
                .accepted
        );
        let old_command = queued.try_recv().unwrap();
        let old_requirement = requirement(&endpoint, (100, 0));
        let epoch = state.lock().unwrap().source_epoch;
        attitude(&state, 0, now - Duration::from_millis(300));
        attitude(&state, 250, now);
        assert_eq!(state.lock().unwrap().source_epoch, epoch.wrapping_add(1));
        link.command_sender().send(old_command).await.unwrap();
        sentinel(&link).await;
        assert_eq!(receive_command(&receiver).await.0, 512);
        assert!(
            endpoint
                .control
                .progress_cached(old_requirement)
                .unwrap()
                .sent_at
                .is_none()
        );
        let mut batch = TelemetryBatch::default();
        endpoint.append_samples(&mut batch);
        assert!(
            batch.samples.is_empty(),
            "old progress cannot acquire the reset camera identity"
        );
        endpoint.maintain();
        assert!(progress_cached(&endpoint, (100, 0)).is_none());
        assert!(
            !endpoint
                .apply_control(&frame(ZoomCommand::Step(1)), SimTick::new(0))
                .action_results[0]
                .accepted
        );
    });
}

#[test]
fn sent_zoom_revocation_keeps_physical_cancellation_unresolved() {
    runtime().block_on(async {
        let (link, receiver) = transport().await;
        let state = link.state();
        let mut endpoint = CameraEndpoints::new(
            VehicleId::new(1),
            state.clone(),
            link.command_sender(),
            1,
            Instant::now(),
        );
        report(&state, Instant::now());
        let mut command = frame(ZoomCommand::Step(1));
        assert!(
            endpoint
                .apply_control(&command, SimTick::new(0))
                .action_results[0]
                .accepted
        );
        assert_eq!(receive_command(&receiver).await.0, 531);
        endpoint.invalidate_pending_commands(VehicleId::new(1), &command.scope);
        command.action_ids = vec![2];
        assert!(
            !endpoint
                .apply_control(&command, SimTick::new(0))
                .action_results[0]
                .accepted
        );
        sentinel(&link).await;
        assert_eq!(receive_command(&receiver).await.0, 512);
        let progress = progress_cached(&endpoint, (100, 0)).unwrap();
        assert!(progress.sent_at.is_some());
        assert_eq!(progress.end_reason, wire::PayloadEndReason::Revoked as i32);
        assert!(progress.observed_complete_at.is_none());
    });
}

async fn queued_deadline(discovery_expires: bool) {
    let (link, receiver) = transport().await;
    let state = link.state();
    let (staging, mut queued) = tokio::sync::mpsc::channel(8);
    let now = Instant::now();
    let received = if discovery_expires {
        now - Duration::from_millis(5_800)
    } else {
        now
    };
    let mut endpoint = CameraEndpoints::new(VehicleId::new(1), state.clone(), staging, 1, now);
    report(&state, received);
    let mut command = frame(ZoomCommand::Step(1));
    command.admitted_timing = Some(pilotage_protocol::AdmittedTiming {
        admitted_at: MonoTimestamp::from_nanos(0),
        send_remaining: Duration::from_millis(if discovery_expires { 500 } else { 100 }),
        active_remaining: None,
        completion_timeout: None,
        window_bounded: true,
    });
    assert!(
        endpoint
            .apply_control(&command, SimTick::new(0))
            .action_results[0]
            .accepted
    );
    let queued_command = queued.try_recv().unwrap();
    let deadline = if discovery_expires {
        received + Duration::from_secs(6)
    } else {
        queued_command.send_before.unwrap()
    };
    assert!(
        tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            std::future::pending::<()>(),
        )
        .await
        .is_err()
    );
    link.command_sender().send(queued_command).await.unwrap();
    sentinel(&link).await;
    assert_eq!(receive_command(&receiver).await.0, 512);
    assert!(
        progress_cached(&endpoint, (100, 0))
            .unwrap()
            .sent_at
            .is_none()
    );
}

#[test]
fn queued_camera_send_deadline_is_enforced_at_actual_transport() {
    runtime().block_on(queued_deadline(false));
}

#[test]
fn queued_camera_discovery_expiry_is_enforced_without_endpoint_poll() {
    runtime().block_on(queued_deadline(true));
}
