//! Camera discovery and zoom use the production adapter's shared MAVLink socket.

use std::time::Duration;

use pilotage_adapter_api::{
    Disposition, LinkLossPolicy, RejectReason, ScopeDescriptor, VehicleAdapter,
};
use pilotage_protocol::{
    ActionKind, ControlAction, ScopedControlFrame, VehicleId, ZoomCommand, ZoomUnit, wire,
};

use super::ArduPilotAdapter;
use super::tests::scoped;

mod socket;
use socket::{ATTACHED, DIRECT, Harness, SECOND_ATTACHED, SYSTEM, float, receive, zoom};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime")
}

fn scope(key: (u8, u8)) -> String {
    format!("vehicle.camera.{}.{}", key.0, key.1)
}

fn command(key: (u8, u8), id: u32, target: i64) -> ScopedControlFrame {
    let mut frame = scoped(
        &scope(key),
        None,
        vec![ControlAction::CameraZoom {
            command: ZoomCommand::Absolute {
                unit: ZoomUnit::TravelFraction,
                value_micros: target,
            },
        }],
    );
    frame.action_ids = vec![id];
    frame
}

fn descriptor(adapter: &ArduPilotAdapter, key: (u8, u8)) -> ScopeDescriptor {
    adapter
        .capabilities()
        .vehicles
        .into_iter()
        .flat_map(|vehicle| vehicle.scopes)
        .find(|descriptor| descriptor.scope.as_str() == scope(key))
        .expect("discovered camera scope")
}

fn payload(adapter: &mut ArduPilotAdapter, key: (u8, u8)) -> wire::PayloadState {
    adapter
        .sample_telemetry()
        .samples
        .into_iter()
        .filter_map(|sample| {
            assert!(
                sample.avionics.is_none(),
                "camera reports must not authorize flight pose"
            );
            sample.payload
        })
        .find(|payload| payload.endpoint_id == scope(key))
        .expect("camera telemetry projection")
}

fn assert_measurement(payload: &wire::PayloadState, key: (u8, u8), value: f64) {
    let zoom = payload.zoom.as_ref().expect("independent measured zoom");
    assert!((zoom.value - value).abs() < 1e-6);
    assert_eq!(zoom.unit, wire::ZoomUnit::TravelFraction as i32);
    assert_eq!(zoom.source, wire::PayloadObservationSource::Measured as i32);
    assert!(
        zoom.sampled_at.is_none(),
        "vehicle boot time is not host time"
    );
    assert!(zoom.received_at.is_some());
    let stamp = zoom.stamp.as_ref().expect("camera provenance");
    assert_eq!(stamp.source_id, u64::from(key.0) * 256 + u64::from(key.1));
    assert_eq!(stamp.role, wire::SourceRole::PayloadDevice as i32);
    assert_eq!(stamp.clock, wire::MeasurementClock::VehicleBoot as i32);
}

fn assert_accepted(adapter: &mut ArduPilotAdapter, frame: &ScopedControlFrame) {
    let outcome = adapter.apply_control(frame);
    assert_eq!(outcome.disposition, Disposition::Accepted);
    assert!(outcome.action_results[0].accepted);
}

async fn assert_sent(harness: &mut Harness, key: (u8, u8), id: u32, target: i64, measured: f64) {
    assert_accepted(&mut harness.adapter, &command(key, id, target));
    let (frame, _) = receive(&harness.socket, zoom).await;
    assert_eq!(&frame[40..42], &[SYSTEM, key.0]);
    assert_eq!(float(&frame, 10), 2.0);
    assert!((float(&frame, 14) - target as f32 / 10_000.0).abs() < 1e-5);
    assert_eq!(
        float(&frame, 18),
        f32::from(if key.1 == 0 { key.0 } else { key.1 })
    );
    let observed = payload(&mut harness.adapter, key);
    assert_measurement(&observed, key, measured);
    let progress = observed.command.expect("public command progress");
    assert_eq!(progress.command_id, u64::from(id));
    assert_eq!(progress.stage, wire::PayloadCommandStage::Sent as i32);
    assert!(progress.sent_at.is_some());
    assert!(progress.acknowledged_at.is_none());
    assert!(progress.observed_complete_at.is_none());
    assert_eq!(progress.zoom_target.unwrap().value_micros, target);
}

#[test]
fn production_start_discovers_camera_scopes_and_routes_zoom_without_a_gimbal_flag() {
    let runtime = runtime();
    runtime.block_on(async {
        let mut harness = Harness::start().await;
        for key in [DIRECT, ATTACHED, SECOND_ATTACHED] {
            let descriptor = descriptor(&harness.adapter, key);
            assert!(
                descriptor.authority_group.is_none(),
                "camera authority is independent"
            );
            assert!(descriptor.axes.is_empty() && descriptor.intents.is_empty());
            assert_eq!(descriptor.actions[0].action, ActionKind::CameraZoom);
            let capability = descriptor.payload.unwrap();
            assert_eq!(capability.resource_scopes, [scope(key)]);
            assert_eq!(capability.connection_incarnation.len(), 16);
            assert_eq!(
                capability.zoom.unwrap().support,
                wire::PayloadSupportState::Available as i32
            );
        }
        assert_sent(&mut harness, DIRECT, 11, 750_000, 0.25).await;
        assert_sent(&mut harness, ATTACHED, 22, 600_000, 0.40).await;
    });
    runtime.shutdown_timeout(Duration::from_secs(1));
}

#[test]
fn camera_lease_loss_fences_a_queued_zoom_without_a_flight_command_or_another_camera_stop() {
    let runtime = runtime();
    runtime.block_on(async {
        let mut harness = Harness::start().await;
        let queued = command(ATTACHED, 31, 750_000);
        assert_accepted(&mut harness.adapter, &queued);
        // A current-thread runtime cannot write the queued frame before these synchronous
        // authority-loss calls complete; the other camera's command is the wire barrier.
        harness
            .adapter
            .invalidate_pending_commands(VehicleId::new(1), &queued.scope);
        harness
            .adapter
            .set_link_loss_policy(
                VehicleId::new(1),
                &queued.scope,
                Some(LinkLossPolicy::Neutralize),
            )
            .expect("camera lease loss enacts");
        let observed = payload(&mut harness.adapter, ATTACHED);
        assert_measurement(&observed, ATTACHED, 0.40);
        let progress = observed.command.unwrap();
        assert_eq!(progress.stage, wire::PayloadCommandStage::Cancelled as i32);
        assert_eq!(progress.end_reason, wire::PayloadEndReason::Revoked as i32);
        assert!(progress.sent_at.is_none());
        assert!(progress.acknowledged_at.is_none() && progress.observed_complete_at.is_none());
        assert_eq!(
            harness.adapter.apply_control(&queued).disposition,
            Disposition::Rejected(RejectReason::LinkLossEngaged)
        );
        assert_sent(&mut harness, SECOND_ATTACHED, 32, 700_000, 0.60).await;
    });
    runtime.shutdown_timeout(Duration::from_secs(1));
}

#[test]
fn changed_camera_capabilities_and_wrong_vehicle_reports_do_not_authorize_zoom() {
    let runtime = runtime();
    runtime.block_on(async {
        let mut harness = Harness::start().await;
        let initial = descriptor(&harness.adapter, DIRECT).payload.unwrap();
        harness.replace_reports(0, true).await;
        let observed = payload(&mut harness.adapter, DIRECT);
        assert_measurement(&observed, DIRECT, 0.25);
        let changed = descriptor(&harness.adapter, DIRECT).payload.unwrap();
        assert!(changed.revision > initial.revision);
        assert_eq!(changed.connection_epoch, initial.connection_epoch);
        assert_eq!(
            changed.connection_incarnation,
            initial.connection_incarnation
        );
        assert_eq!(
            changed.zoom.unwrap().support,
            wire::PayloadSupportState::Unsupported as i32
        );
        assert!(
            harness
                .adapter
                .capabilities()
                .vehicles
                .iter()
                .flat_map(|vehicle| &vehicle.scopes)
                .all(|descriptor| descriptor.scope.as_str() != scope((101, 0)))
        );
        let refused = harness.adapter.apply_control(&command(DIRECT, 41, 750_000));
        assert!(!refused.action_results[0].accepted);
        assert_sent(&mut harness, ATTACHED, 42, 600_000, 0.40).await;
    });
    runtime.shutdown_timeout(Duration::from_secs(1));
}
