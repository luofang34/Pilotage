use super::*;
use pilotage_adapter_api::Disposition;
use pilotage_mavlink::{FcMessage, FrameSource, codec::CameraMessage};
use pilotage_protocol::{ControlAction, ControlPayload, ZoomCommand};
use std::time::Instant;

fn camera_adapter() -> (
    Px4Adapter,
    tokio::sync::mpsc::Receiver<pilotage_mavlink::OutboundCommand>,
) {
    let state = Arc::new(Mutex::new(LinkState::default()));
    let (sender, receiver) = tokio::sync::mpsc::channel(8);
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), state.clone());
    adapter.camera_control = Some(CameraEndpoints::new(
        VehicleId::new(1),
        state.clone(),
        sender,
        1,
        adapter.started_at,
    ));
    pilotage_mavlink::link::apply_messages_at(
        &state,
        &[(
            FrameSource {
                system_id: 1,
                component_id: 100,
                frame_sequence: 0,
            },
            FcMessage::Camera(CameraMessage::Information {
                time_boot_ms: 100,
                flags: 64,
                device_id: 0,
                gimbal_id: 0,
            }),
        )],
        0,
        0,
        Instant::now(),
    );
    (adapter, receiver)
}

fn zoom_frame() -> ScopedControlFrame {
    let mut frame = tests::frame(Vec::new(), Vec::new());
    frame.scope = ScopeId::new("vehicle.camera.100.0");
    frame.intent = None;
    frame.payload = ControlPayload::default();
    frame.actions = vec![ControlAction::CameraZoom {
        command: ZoomCommand::Step(1),
    }];
    frame.action_ids = vec![1];
    frame
}

#[test]
fn camera_scope_routes_and_reports_independently_from_flight() {
    let (mut adapter, mut receiver) = camera_adapter();
    let frame = zoom_frame();
    let capabilities = adapter.capabilities();
    assert_eq!(capabilities.vehicles[0].scopes.len(), 1);
    assert_eq!(capabilities.vehicles[0].scopes[0].scope, frame.scope);
    let outcome = adapter.apply_control(&frame);
    assert_eq!(outcome.disposition, Disposition::Accepted);
    assert!(outcome.action_results[0].accepted);
    assert_eq!(receiver.try_recv().unwrap().command, 531);
    adapter.invalidate_pending_commands(VehicleId::new(1), &frame.scope);
    let batch = adapter.sample_telemetry();
    let payload = batch
        .samples
        .iter()
        .find_map(|s| s.payload.as_ref())
        .unwrap();
    assert_eq!(payload.endpoint_id, frame.scope.as_str());
    let progress = payload.command.as_ref().unwrap();
    assert_eq!(
        progress.stage,
        pilotage_protocol::wire::PayloadCommandStage::Cancelled as i32
    );
    assert!(progress.sent_at.is_none());
    assert!(batch.samples.iter().all(|s| s.fc_state.is_none()));
}

#[test]
fn camera_link_loss_latch_is_scoped_and_revokes_queued_control() {
    let (mut adapter, mut receiver) = camera_adapter();
    let frame = zoom_frame();
    assert!(adapter.apply_control(&frame).action_results[0].accepted);
    assert_eq!(receiver.try_recv().unwrap().command, 531);
    adapter
        .enact_link_loss_policy(
            VehicleId::new(1),
            &frame.scope,
            Some(LinkLossPolicy::Neutralize),
        )
        .unwrap();
    assert_eq!(
        adapter.apply_control(&frame).disposition,
        Disposition::Rejected(RejectReason::LinkLossEngaged),
    );
    assert!(!pilotage_adapter_api::link_loss_latch_engaged(
        &adapter.link_loss_policy,
        &ScopeId::new(FLIGHT_SCOPE),
    ));
    let batch = adapter.sample_telemetry();
    let progress = batch
        .samples
        .iter()
        .find_map(|s| s.payload.as_ref()?.command.as_ref())
        .unwrap();
    assert_eq!(
        progress.end_reason,
        pilotage_protocol::wire::PayloadEndReason::Revoked as i32
    );
    assert!(progress.sent_at.is_none());
}
