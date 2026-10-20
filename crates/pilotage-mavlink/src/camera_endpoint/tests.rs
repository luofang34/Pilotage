use super::*;
use crate::{FcMessage, FrameSource, LinkState, codec::CameraMessage};
use pilotage_adapter_api::SourceIncarnation;
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
    report_camera(state, (100, 0), 0.25, now);
}

fn report_camera(state: &Arc<Mutex<LinkState>>, key: (u8, u8), zoom: f32, now: Instant) {
    let source = FrameSource {
        system_id: 1,
        component_id: key.0,
        frame_sequence: 0,
    };
    crate::link::apply_messages_at(
        state,
        &[
            (
                source,
                FcMessage::Camera(CameraMessage::Information {
                    time_boot_ms: 100,
                    flags: 64,
                    device_id: key.1,
                    gimbal_id: 0,
                }),
            ),
            (
                source,
                FcMessage::Camera(CameraMessage::Settings {
                    time_boot_ms: 101,
                    device_id: key.1,
                    travel_fraction: Some(zoom),
                }),
            ),
        ],
        0,
        0,
        now,
    );
}

fn setting(state: &Arc<Mutex<LinkState>>, time: u32, zoom: f32) {
    crate::link::apply_messages_at(
        state,
        &[(
            FrameSource {
                system_id: 1,
                component_id: 100,
                frame_sequence: 1,
            },
            FcMessage::Camera(CameraMessage::Settings {
                time_boot_ms: time,
                device_id: 0,
                travel_fraction: Some(zoom),
            }),
        )],
        0,
        0,
        Instant::now(),
    );
}

#[test]
fn discovered_camera_has_own_identity_scope_and_typed_command_route() {
    let state = Arc::new(Mutex::new(LinkState {
        source_incarnation: SourceIncarnation::new([4; 16]),
        ..Default::default()
    }));
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    let mut endpoint =
        CameraEndpoints::new(VehicleId::new(1), state.clone(), sender, 1, Instant::now());
    assert!(endpoint.scopes().is_empty());
    report(&state, Instant::now());
    let scopes = endpoint.scopes();
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
    let result = endpoint.apply_control(&command, SimTick::new(0));
    assert_eq!(result.disposition, Disposition::Accepted);
    let queued = receiver.try_recv().unwrap();
    assert_eq!(queued.command, 531);
    assert_eq!(queued.target_component, 100);
    assert_eq!(queued.params[..3], [2.0, 75.0, 100.0]);
    assert!(queued.payload_write.is_some());
    let progress = progress_cached(&endpoint, (100, 0)).unwrap();
    assert!(progress.sent_at.is_none());
    assert_eq!(progress.zoom_target.unwrap().value_micros, 750_000);
    endpoint.invalidate_pending_commands(VehicleId::new(1), &command.scope);
    let progress = progress_cached(&endpoint, (100, 0)).unwrap();
    assert_eq!(progress.stage, wire::PayloadCommandStage::Cancelled as i32);
    let batch = sample(&mut endpoint);
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
    let mut endpoint =
        CameraEndpoints::new(VehicleId::new(1), state.clone(), sender, 1, Instant::now());
    report(&state, Instant::now() - Duration::from_secs(7));
    endpoint.apply_control(&frame(ZoomCommand::Step(1)), SimTick::new(0));
    assert!(receiver.try_recv().is_err());
    let payload = endpoint.scopes().remove(0).payload.unwrap();
    assert_eq!(
        payload.zoom.unwrap().support,
        wire::PayloadSupportState::Unavailable as i32
    );
}

#[test]
fn undeclared_zoom_operations_never_enter_command_queue() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    let mut endpoint =
        CameraEndpoints::new(VehicleId::new(1), state.clone(), sender, 1, Instant::now());
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
        let outcome = endpoint.apply_control(&frame(command), SimTick::new(0));
        assert!(!outcome.action_results[0].accepted, "{command:?}");
    }
    assert!(receiver.try_recv().is_err());
    assert!(progress_cached(&endpoint, (100, 0)).is_none());
}

#[test]
fn exhausted_send_budget_and_full_queue_do_not_claim_command_admission() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    let mut endpoint = CameraEndpoints::new(
        VehicleId::new(1),
        state.clone(),
        sender.clone(),
        1,
        Instant::now(),
    );
    report(&state, Instant::now());
    let mut expired = frame(ZoomCommand::Step(1));
    expired.admitted_timing = Some(pilotage_protocol::AdmittedTiming {
        admitted_at: MonoTimestamp::from_nanos(0),
        send_remaining: Duration::ZERO,
        active_remaining: None,
        completion_timeout: None,
        window_bounded: true,
    });
    assert!(
        !endpoint
            .apply_control(&expired, SimTick::new(0))
            .action_results[0]
            .accepted
    );
    assert!(receiver.try_recv().is_err());
    let permit = sender.try_reserve().unwrap();
    assert!(
        !endpoint
            .apply_control(&frame(ZoomCommand::Step(1)), SimTick::new(0))
            .action_results[0]
            .accepted
    );
    assert!(progress_cached(&endpoint, (100, 0)).is_none());
    drop(permit);
}

fn sample(endpoint: &mut CameraEndpoints) -> TelemetryBatch {
    let mut batch = TelemetryBatch::default();
    endpoint.sample_telemetry(&mut batch);
    batch
}

fn requirement(endpoint: &CameraEndpoints, key: (u8, u8)) -> CameraWriteRequirement {
    let state = endpoint.state.lock().unwrap();
    camera_requirement(&state, key, state.fc_reports.cameras.get(&key).unwrap())
}

fn progress_cached(
    endpoint: &CameraEndpoints,
    key: (u8, u8),
) -> Option<wire::PayloadCommandProgress> {
    endpoint.control.progress_cached(requirement(endpoint, key))
}

#[test]
fn camera_scope_aliases_cannot_admit_or_revoke_commands() {
    let state = Arc::new(Mutex::new(LinkState::default()));
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    let mut endpoint =
        CameraEndpoints::new(VehicleId::new(1), state.clone(), sender, 1, Instant::now());
    report(&state, Instant::now());
    let canonical = frame(ZoomCommand::Step(1));
    assert!(
        endpoint
            .apply_control(&canonical, SimTick::new(0))
            .action_results[0]
            .accepted
    );
    receiver.try_recv().unwrap();
    let before = progress_cached(&endpoint, (100, 0)).unwrap();
    for alias in [
        "vehicle.camera.0100.0",
        "vehicle.camera.+100.0",
        "vehicle.camera.100.00",
    ] {
        let mut command = canonical.clone();
        command.scope = ScopeId::new(alias);
        assert!(matches!(
            endpoint
                .apply_control(&command, SimTick::new(0))
                .disposition,
            Disposition::Rejected(_),
        ));
        endpoint.invalidate_pending_commands(VehicleId::new(1), &command.scope);
        assert_eq!(progress_cached(&endpoint, (100, 0)).unwrap(), before);
    }
    assert!(receiver.try_recv().is_err());
    endpoint.invalidate_pending_commands(VehicleId::new(1), &canonical.scope);
    assert_eq!(
        progress_cached(&endpoint, (100, 0)).unwrap().stage,
        wire::PayloadCommandStage::Cancelled as i32,
    );
}
