//! Unit tests for the Gazebo `VehicleAdapter` implementation in `adapter.rs`:
//! axis clamping and mapping, telemetry conversion, and end-to-end control /
//! odometry / camera flow against an in-process fake bridge on a loopback
//! socket (no child process, no live Gazebo).
#![allow(clippy::expect_used, clippy::panic)]

use super::{
    GazeboAdapter, MAX_ANGULAR_RPS, MAX_LINEAR_MPS, MOTION_SCOPE, clamp_axis, control_from_intent,
    telemetry_from_odometry,
};
use crate::bridge_client::BridgeClient;
use crate::error::GazeboAdapterError;
use crate::framing::read_envelope;
use crate::wire::{BridgeEnvelope, BridgeFrame, BridgeOdometry, bridge_envelope};
use pilotage_adapter_api::{Disposition, RejectReason, VehicleAdapter};
use pilotage_protocol::{
    ControlIntent, ControlPayload, Generation, ReferenceFrame, ScopeId, ScopedControlFrame,
    SequenceNum, SessionId, VehicleId, VelocityIntent,
};
use pilotage_timing::MonoTimestamp;
use prost::Message;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

fn velocity(vx: f32, yaw_rate: f32) -> VelocityIntent {
    VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx,
        vy: 0.0,
        vz: 0.0,
        yaw_rate,
    }
}

fn frame(scope: &str, intent: Option<ControlIntent>, vehicle: VehicleId) -> ScopedControlFrame {
    ScopedControlFrame {
        action_ids: vec![],
        session: SessionId::new(1),
        vehicle,
        scope: ScopeId::new(scope),
        generation: Generation::new(1),
        sequence: SequenceNum::new(1),
        sampled_at: MonoTimestamp::from_nanos(0),
        profile_revision: 1,
        activation_revision: 0,
        payload: ControlPayload::default(),
        intent,
        actions: vec![],
    }
}

/// Binds a loopback listener and returns both ends of an accepted stream,
/// standing in for the sidecar<->host socket without any child process.
async fn connected_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind loopback listener");
    let addr = listener.local_addr().expect("listener addr");
    let connect = tokio::spawn(async move { TcpStream::connect(addr).await });
    let (host_side, _peer) = listener.accept().await.expect("accept fake bridge");
    let bridge_side = connect.await.expect("join connect").expect("connect");
    (host_side, bridge_side)
}

async fn send_envelope(stream: &mut TcpStream, envelope: &BridgeEnvelope) {
    let bytes = envelope.encode_length_delimited_to_vec();
    stream.write_all(&bytes).await.expect("fake bridge write");
}

#[test]
fn clamp_axis_neutralizes_nan_and_bounds_infinity() {
    assert_eq!(clamp_axis(f64::NAN), (0.0, true));
    assert_eq!(clamp_axis(2.0), (1.0, true));
    assert_eq!(clamp_axis(-5.0), (-1.0, true));
    assert_eq!(clamp_axis(0.5), (0.5, false));
}

#[test]
fn control_maps_typed_velocity_to_twist() {
    let (control, constrained) =
        control_from_intent(&velocity(0.8 * MAX_LINEAR_MPS, -0.4 * MAX_ANGULAR_RPS));
    assert!(!constrained);
    assert!((control.linear_x - 0.8).abs() < 1e-6);
    assert!((control.angular_z - -0.4).abs() < 1e-6);
}

#[test]
fn control_clamps_out_of_envelope_and_reports_constrained() {
    let (control, constrained) = control_from_intent(&velocity(9.0, 0.0));
    assert!(constrained);
    assert!((control.linear_x - 1.0).abs() < 1e-6);
}

/// A lateral component a diff-drive cannot execute is constrained, and the
/// executable components still apply.
#[test]
fn control_constrains_an_inexecutable_lateral_component() {
    let mut intent = velocity(0.5, 0.0);
    intent.vy = 0.5;
    let (control, constrained) = control_from_intent(&intent);
    assert!(constrained);
    assert!((control.linear_x - 0.5).abs() < 1e-6);
}

#[test]
fn odometry_maps_to_canonical_telemetry() {
    let sample = telemetry_from_odometry(
        VehicleId::new(3),
        &BridgeOdometry {
            x: 1.0,
            y: 2.0,
            heading: 0.5,
            speed: 4.0,
            sim_time_ns: 900,
        },
    );
    assert_eq!(sample.vehicle, VehicleId::new(3));
    assert_eq!(sample.tick.as_u64(), 900);
    assert!((sample.pose.expect("pose").x - 1.0).abs() < 1e-6);
    assert!((sample.speed.expect("speed") - 4.0).abs() < 1e-6);
}

#[tokio::test]
async fn apply_control_sends_mapped_command_over_the_bridge() {
    let vehicle = VehicleId::new(1);
    let (host_side, mut bridge_side) = connected_pair().await;
    let mut adapter =
        GazeboAdapter::from_bridge(vehicle, BridgeClient::connect_stream_for_test(host_side));

    let outcome = adapter.apply_control(&frame(
        MOTION_SCOPE,
        Some(ControlIntent::Velocity(velocity(0.5, 0.25))),
        vehicle,
    ));
    assert_eq!(outcome.disposition, Disposition::Accepted);

    let received = read_envelope(&mut bridge_side)
        .await
        .expect("fake bridge reads control")
        .expect("control envelope present");
    match received.payload {
        Some(bridge_envelope::Payload::Control(control)) => {
            assert!((control.linear_x - 0.5).abs() < 1e-6);
            assert!((control.angular_z - 0.25).abs() < 1e-6);
        }
        other => panic!("expected a control payload, got {other:?}"),
    }
}

#[tokio::test]
async fn unknown_scope_is_rejected_without_sending() {
    let vehicle = VehicleId::new(1);
    let (host_side, _bridge_side) = connected_pair().await;
    let mut adapter =
        GazeboAdapter::from_bridge(vehicle, BridgeClient::connect_stream_for_test(host_side));
    let outcome = adapter.apply_control(&frame("vehicle.camera", None, vehicle));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::UnknownScope)
    );
}

/// The typed-only contract: a legacy numeric payload reaching the adapter is
/// a session-boundary violation, rejected rather than interpreted.
#[tokio::test]
async fn a_legacy_payload_frame_is_rejected() {
    use pilotage_protocol::LogicalAxisId;
    let vehicle = VehicleId::new(1);
    let (host_side, _bridge_side) = connected_pair().await;
    let mut adapter =
        GazeboAdapter::from_bridge(vehicle, BridgeClient::connect_stream_for_test(host_side));
    let mut legacy = frame(MOTION_SCOPE, None, vehicle);
    legacy.payload = ControlPayload {
        axes: vec![(LogicalAxisId::new(super::THROTTLE_AXIS), 1.0)],
        edges: vec![],
    };
    let outcome = adapter.apply_control(&legacy);
    assert!(matches!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::Other(_))
    ));
}

#[tokio::test]
async fn cached_odometry_becomes_telemetry_sample() {
    let vehicle = VehicleId::new(2);
    let (host_side, mut bridge_side) = connected_pair().await;
    let mut adapter =
        GazeboAdapter::from_bridge(vehicle, BridgeClient::connect_stream_for_test(host_side));

    send_envelope(
        &mut bridge_side,
        &BridgeEnvelope {
            payload: Some(bridge_envelope::Payload::Odometry(BridgeOdometry {
                x: 5.0,
                y: -1.0,
                heading: 1.5,
                speed: 2.5,
                sim_time_ns: 1234,
            })),
        },
    )
    .await;

    // Poll until the reader task has published the odometry.
    let mut telemetry = adapter.sample_telemetry();
    for _ in 0..200 {
        if !telemetry.samples.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        telemetry = adapter.sample_telemetry();
    }
    let sample = telemetry.samples.first().expect("telemetry sample present");
    assert_eq!(sample.tick.as_u64(), 1234);
    assert!((sample.pose.expect("pose").x - 5.0).abs() < 1e-6);
    assert!((sample.speed.expect("speed") - 2.5).abs() < 1e-6);
}

#[tokio::test]
async fn camera_frame_reaches_the_subscriber() {
    let vehicle = VehicleId::new(4);
    let (host_side, mut bridge_side) = connected_pair().await;
    let mut adapter =
        GazeboAdapter::from_bridge(vehicle, BridgeClient::connect_stream_for_test(host_side));
    let mut frames = adapter.subscribe_frames().expect("frame receiver present");

    send_envelope(
        &mut bridge_side,
        &BridgeEnvelope {
            payload: Some(bridge_envelope::Payload::Frame(BridgeFrame {
                width: 4,
                height: 2,
                pixel_format: "RGB_INT8".to_owned(),
                sim_time_ns: 77,
                rgb: vec![9_u8; 24],
                camera_id: 0,
            })),
        },
    )
    .await;

    let frame = frames.recv().await.expect("a frame arrives");
    assert_eq!(frame.source_id, 0, "camera_id 0 maps to the FPV source id");
    assert_eq!(frame.width, 4);
    assert_eq!(frame.height, 2);
    assert_eq!(frame.tick.as_u64(), 77);
    assert_eq!(frame.rgb.len(), 24);
    // The adapter stamps a capture identity: the sidecar sim time becomes the
    // capture acquisition time, and the sim clock maps to itself (ADR-0020).
    assert_eq!(frame.capture.stamp.acquired_at_ns, 77);
    assert_eq!(
        frame.capture.stamp.sequence, 0,
        "first FPV frame is sequence 0"
    );
    assert!(
        frame.capture.mapping.is_available(),
        "sim capture clock maps to sim telemetry clock"
    );
}

#[tokio::test]
async fn chase_camera_frame_carries_its_source_id() {
    let vehicle = VehicleId::new(5);
    let (host_side, mut bridge_side) = connected_pair().await;
    let mut adapter =
        GazeboAdapter::from_bridge(vehicle, BridgeClient::connect_stream_for_test(host_side));
    let mut frames = adapter.subscribe_frames().expect("frame receiver present");

    send_envelope(
        &mut bridge_side,
        &BridgeEnvelope {
            payload: Some(bridge_envelope::Payload::Frame(BridgeFrame {
                width: 2,
                height: 2,
                pixel_format: "RGB_INT8".to_owned(),
                sim_time_ns: 88,
                rgb: vec![7_u8; 12],
                camera_id: 1,
            })),
        },
    )
    .await;

    let frame = frames.recv().await.expect("a chase frame arrives");
    assert_eq!(
        frame.source_id, 1,
        "camera_id 1 maps to the chase source id"
    );
    assert_eq!(frame.tick.as_u64(), 88);
}

#[tokio::test]
async fn reader_death_surfaces_liveness_and_withholds_stale_telemetry() {
    let vehicle = VehicleId::new(6);
    let (host_side, mut bridge_side) = connected_pair().await;
    let mut adapter =
        GazeboAdapter::from_bridge(vehicle, BridgeClient::connect_stream_for_test(host_side));

    // Seed a cached odometry sample so there is a "last value" to go stale.
    send_envelope(
        &mut bridge_side,
        &BridgeEnvelope {
            payload: Some(bridge_envelope::Payload::Odometry(BridgeOdometry {
                x: 3.0,
                y: 3.0,
                heading: 0.0,
                speed: 1.0,
                sim_time_ns: 500,
            })),
        },
    )
    .await;
    let mut telemetry = adapter.sample_telemetry();
    for _ in 0..200 {
        if !telemetry.samples.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        telemetry = adapter.sample_telemetry();
    }
    assert!(!telemetry.samples.is_empty(), "odometry should be cached");
    assert!(adapter.reader_health().is_ok(), "reader alive before EOF");

    // Drop the bridge end: the reader hits EOF and must publish its death.
    drop(bridge_side);
    let mut health = adapter.reader_health();
    for _ in 0..200 {
        if health.is_err() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        health = adapter.reader_health();
    }
    assert!(
        matches!(health, Err(GazeboAdapterError::ReaderTaskEnded { .. })),
        "reader death must surface as ReaderTaskEnded, got {health:?}"
    );
    // The stale sample must no longer be presented as live telemetry.
    assert!(
        adapter.sample_telemetry().samples.is_empty(),
        "telemetry must be withheld once the reader is dead"
    );
}

#[tokio::test]
async fn set_link_loss_policy_sends_stop() {
    use pilotage_adapter_api::LinkLossPolicy;
    let vehicle = VehicleId::new(9);
    let (host_side, mut bridge_side) = connected_pair().await;
    let mut adapter =
        GazeboAdapter::from_bridge(vehicle, BridgeClient::connect_stream_for_test(host_side));

    adapter
        .set_link_loss_policy(
            vehicle,
            &ScopeId::new(MOTION_SCOPE),
            Some(LinkLossPolicy::Neutralize),
        )
        .expect("policy enacted");
    let received = read_envelope(&mut bridge_side)
        .await
        .expect("stop is read")
        .expect("stop envelope present");
    match received.payload {
        Some(bridge_envelope::Payload::Control(control)) => {
            assert_eq!(control.linear_x, 0.0);
            assert_eq!(control.angular_z, 0.0);
        }
        other => panic!("expected a control payload, got {other:?}"),
    }
}

#[tokio::test]
async fn capabilities_report_motion_scope_and_camera_source() {
    let vehicle = VehicleId::new(1);
    let (host_side, _bridge_side) = connected_pair().await;
    let adapter =
        GazeboAdapter::from_bridge(vehicle, BridgeClient::connect_stream_for_test(host_side));
    let caps = adapter.capabilities();
    assert!(caps.execution.real_time);
    assert!(caps.execution.render_capable);
    assert_eq!(caps.vehicles.len(), 1);
    assert_eq!(caps.vehicles[0].scopes[0].scope.as_str(), MOTION_SCOPE);
    let sources = adapter.video_sources();
    assert_eq!(sources.len(), 2, "FPV and chase are both advertised");
    assert_eq!(sources[0].id, super::FPV_SOURCE_ID);
    assert_eq!(sources[1].id, super::CHASE_SOURCE_ID);
}
