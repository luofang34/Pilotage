use super::*;
use pilotage_adapter_api::{SourceIncarnation, VehicleAdapter};
use pilotage_mavlink::{FcMessage, FrameSource, LinkState, codec::CameraMessage};
use pilotage_protocol::{
    ControlPayload, Generation, ScopeId, SequenceNum, SessionId, VehicleId, ZoomCommand, ZoomUnit,
};
use pilotage_timing::MonoTimestamp;
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod transport;

fn frame(command: ZoomCommand) -> ScopedControlFrame {
    ScopedControlFrame {
        timing: None,
        admitted_timing: None,
        session: SessionId::new(1),
        vehicle: VehicleId::new(1),
        scope: ScopeId::new("vehicle.camera.100.0"),
        generation: Generation::new(1),
        sequence: SequenceNum::new(1),
        sampled_at: MonoTimestamp::from_nanos(0),
        profile_revision: 1,
        activation_revision: 1,
        payload: ControlPayload::default(),
        intent: None,
        actions: vec![ControlAction::CameraZoom { command }],
        action_ids: vec![1],
    }
}

fn report(state: &Arc<Mutex<LinkState>>, now: Instant) {
    let source = FrameSource {
        system_id: 1,
        component_id: 100,
        frame_sequence: 0,
    };
    pilotage_mavlink::link::apply_messages_at(
        state,
        &[
            (
                source,
                FcMessage::Camera(CameraMessage::Information {
                    time_boot_ms: 100,
                    flags: 64,
                    device_id: 0,
                    gimbal_id: 0,
                }),
            ),
            (
                source,
                FcMessage::Camera(CameraMessage::Settings {
                    time_boot_ms: 101,
                    device_id: 0,
                    travel_fraction: Some(0.25),
                }),
            ),
        ],
        0,
        0,
        now,
    );
}

#[test]
fn discovered_camera_has_own_identity_scope_and_typed_command_route() {
    let state = Arc::new(Mutex::new(LinkState {
        source_incarnation: SourceIncarnation::new([4; 16]),
        ..Default::default()
    }));
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), state.clone());
    adapter.camera_control = Some(CameraControl::new(sender, 1));
    assert!(adapter.camera_scopes().is_empty());
    report(&state, Instant::now());
    let scopes = adapter.camera_scopes();
    let payload = scopes[0].payload.as_ref().unwrap();
    assert_eq!(payload.resource_scopes, ["vehicle.camera.100.0"]);
    assert_eq!(payload.connection_incarnation, vec![4; 16]);
    assert_eq!(payload.connection_epoch, 1);
    assert!(!pilotage_protocol::payload_zoom_admits(
        payload.zoom.as_ref(),
        ZoomCommand::Continuous(1)
    ));
    let command = frame(ZoomCommand::Absolute {
        unit: ZoomUnit::TravelFraction,
        value_micros: 750_000,
    });
    let result = adapter.apply_control(&command);
    assert_eq!(result.disposition, Disposition::Accepted);
    let queued = receiver.try_recv().unwrap();
    assert_eq!(queued.command, 531);
    assert_eq!(queued.target_component, 100);
    assert_eq!(queued.params[..3], [2.0, 75.0, 100.0]);
    assert!(queued.payload_write.is_some());
    let progress = adapter
        .camera_control
        .as_ref()
        .unwrap()
        .progress_cached((100, 0))
        .unwrap();
    assert!(progress.sent_at.is_none());
    assert_eq!(progress.zoom_target.unwrap().value_micros, 750_000);
    adapter.invalidate_pending_commands(VehicleId::new(1), &command.scope);
    let progress = adapter
        .camera_control
        .as_ref()
        .unwrap()
        .progress_cached((100, 0))
        .unwrap();
    assert_eq!(progress.stage, wire::PayloadCommandStage::Cancelled as i32);
    let batch = adapter.sample_telemetry();
    let measured = batch
        .samples
        .iter()
        .find_map(|sample| sample.payload.as_ref()?.zoom.as_ref())
        .unwrap();
    assert_eq!(
        measured.value, 0.25,
        "a commanded target cannot replace the actual setting"
    );
    assert_eq!(
        measured.sampled_at, None,
        "device boot time is not host monotonic time"
    );
}

#[test]
fn expired_discovery_refuses_controls_and_advertises_unavailability() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), state.clone());
    adapter.camera_control = Some(CameraControl::new(sender, 1));
    report(&state, Instant::now() - Duration::from_secs(7));
    adapter.apply_control(&frame(ZoomCommand::Step(1)));
    assert!(receiver.try_recv().is_err());
    let payload = adapter.camera_scopes().remove(0).payload.unwrap();
    assert_eq!(
        payload.zoom.unwrap().support,
        wire::PayloadSupportState::Unavailable as i32
    );
}

#[test]
fn undeclared_zoom_operations_never_enter_command_queue() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), state.clone());
    adapter.camera_control = Some(CameraControl::new(sender, 1));
    report(&state, Instant::now());
    for command in [
        ZoomCommand::Absolute {
            unit: ZoomUnit::Ratio,
            value_micros: 2_000_000,
        },
        ZoomCommand::Absolute {
            unit: ZoomUnit::FocalLengthMm,
            value_micros: 35_000_000,
        },
        ZoomCommand::Absolute {
            unit: ZoomUnit::HorizontalFovRadians,
            value_micros: 1_000_000,
        },
        ZoomCommand::Absolute {
            unit: ZoomUnit::TravelFraction,
            value_micros: 1_000_001,
        },
        ZoomCommand::Continuous(100_000),
        ZoomCommand::Step(2),
        ZoomCommand::Step(0),
    ] {
        let outcome = adapter.apply_control(&frame(command));
        assert!(!outcome.action_results[0].accepted, "{command:?}");
    }
    assert!(receiver.try_recv().is_err());
    assert!(
        adapter
            .camera_control
            .as_ref()
            .unwrap()
            .progress_cached((100, 0))
            .is_none()
    );
}

#[test]
fn exhausted_send_budget_and_full_queue_do_not_claim_command_admission() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), state.clone());
    adapter.camera_control = Some(CameraControl::new(sender.clone(), 1));
    report(&state, Instant::now());
    let mut expired = frame(ZoomCommand::Step(1));
    expired.admitted_timing = Some(pilotage_protocol::AdmittedTiming {
        admitted_at: MonoTimestamp::from_nanos(0),
        send_remaining: Duration::ZERO,
        active_remaining: None,
        completion_timeout: None,
        window_bounded: true,
    });
    assert!(!adapter.apply_control(&expired).action_results[0].accepted);
    assert!(receiver.try_recv().is_err());
    let permit = sender.try_reserve().unwrap();
    assert!(
        !adapter
            .apply_control(&frame(ZoomCommand::Step(1)))
            .action_results[0]
            .accepted
    );
    assert!(
        adapter
            .camera_control
            .as_ref()
            .unwrap()
            .progress_cached((100, 0))
            .is_none()
    );
    drop(permit);
}
