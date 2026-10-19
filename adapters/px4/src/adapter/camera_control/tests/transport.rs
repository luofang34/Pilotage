use super::*;
use pilotage_mavlink::{CommandTarget, LinkConfig, MavlinkLink, OutboundCommand};
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
        let mut adapter = Px4Adapter::from_state(VehicleId::new(1), state.clone());
        adapter.camera_control = Some(CameraControl::new(link.command_sender(), 1));
        report(&state, Instant::now());
        let command = frame(ZoomCommand::Absolute {
            unit: ZoomUnit::TravelFraction,
            value_micros: 750_000,
        });
        adapter.apply_control(&command);
        let (id, bytes) = receive_command(&receiver).await;
        assert_eq!(id, 531);
        assert_eq!(f32::from_le_bytes(bytes[14..18].try_into().unwrap()), 75.0);
        let progress = adapter
            .camera_control
            .as_ref()
            .unwrap()
            .progress_cached((100, 0))
            .unwrap();
        assert_eq!(progress.stage, wire::PayloadCommandStage::Sent as i32);
        assert!(progress.sent_at.is_some());
        assert!(progress.acknowledged_at.is_none());
        assert!(progress.observed_complete_at.is_none());
        let sampled = adapter.sample_telemetry();
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
    });
}

async fn assert_queued_command_fenced(change_capability: bool) {
    let (link, receiver) = transport().await;
    let state = link.state();
    let (staging, mut queued) = tokio::sync::mpsc::channel(8);
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), state.clone());
    adapter.camera_control = Some(CameraControl::new(staging, 1));
    report(&state, Instant::now());
    let command = frame(ZoomCommand::Step(1));
    adapter.apply_control(&command);
    let stale = queued.try_recv().unwrap();
    if change_capability {
        pilotage_mavlink::link::apply_messages_at(
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
        adapter.invalidate_pending_commands(VehicleId::new(1), &command.scope);
    }
    link.command_sender().send(stale).await.unwrap();
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
    assert_eq!(
        receive_command(&receiver).await.0,
        512,
        "the ordered sentinel must arrive without a stale zoom before it"
    );
    adapter.maintain_camera();
    let progress = adapter
        .camera_control
        .as_ref()
        .unwrap()
        .progress_cached((100, 0))
        .unwrap();
    assert!(progress.sent_at.is_none());
    assert_eq!(progress.end_reason, wire::PayloadEndReason::Revoked as i32);
}

#[test]
fn revoked_queued_zoom_cannot_reach_actual_transport() {
    runtime().block_on(assert_queued_command_fenced(false));
}

#[test]
fn raw_camera_capability_change_fences_zoom_before_adapter_synchronizes() {
    runtime().block_on(assert_queued_command_fenced(true));
}
