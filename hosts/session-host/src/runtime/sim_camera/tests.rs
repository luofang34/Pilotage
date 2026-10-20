#![allow(clippy::expect_used, clippy::panic)]

use std::f64::consts::FRAC_PI_2;
use std::time::Duration;

use pilotage_adapter_api::{CaptureClockMapping, MeasurementClock};
use pilotage_protocol::wire;
use pilotage_sim_video::read_envelope;
use pilotage_sim_video::wire::{BridgeEnvelope, BridgeFrame, bridge_envelope::Payload};
use prost::Message;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use super::pose::truth_pose;
use super::{SIM_CAMERA_CLIENT, spawn};
use crate::runtime::connection::{DatagramClass, ToConnection};
use crate::runtime::engine_actor::ToEngine;
use crate::runtime::wire_codec::encode_telemetry_datagram;

const N51: (f64, f64, f64) = (40.577_186, -74.738_192, 176.3);
/// A failed delivery fails the test at this deadline instead of hanging it.
const DEADLINE: Duration = Duration::from_secs(10);

/// A quaternion for a yaw (about down) then a pitch (about right).
fn yaw_pitch(yaw: f64, pitch: f64) -> [f32; 4] {
    let (sy, cy) = (yaw / 2.0).sin_cos();
    let (sp, cp) = (pitch / 2.0).sin_cos();
    [cy * cp, -sy * sp, cy * sp, sy * cp].map(|value| value as f32)
}

fn stamp(role: wire::SourceRole, acquired_at_ns: u64) -> wire::MeasurementStamp {
    wire::MeasurementStamp {
        acquired_at_ns,
        clock: wire::MeasurementClock::Simulation as i32,
        role: role as i32,
        ..wire::MeasurementStamp::default()
    }
}

fn truth_sample(acquired_at_ns: u64, yaw: f64) -> wire::TelemetrySample {
    let quat = yaw_pitch(yaw, 0.0);
    wire::TelemetrySample {
        sim_truth: Some(Box::new(wire::SimTruthState {
            quat_w: quat[0],
            quat_x: quat[1],
            quat_y: quat[2],
            quat_z: quat[3],
            valid_flags: 0b1111,
            geodetic: Some(wire::GeodeticFix {
                latitude_deg: N51.0,
                longitude_deg: N51.1,
                horizontal_datum: 1,
                height_m: N51.2,
                vertical_datum: 2,
                ..wire::GeodeticFix::default()
            }),
            stamp: Some(stamp(wire::SourceRole::SimulationTruth, acquired_at_ns)),
            ..wire::SimTruthState::default()
        })),
        ..wire::TelemetrySample::default()
    }
}

fn gimbal(yaw: f64, pitch: f64, frame: wire::GimbalYawFrame) -> wire::GimbalAttitude {
    let quat = yaw_pitch(yaw, pitch);
    wire::GimbalAttitude {
        quat_w: quat[0],
        quat_x: quat[1],
        quat_y: quat[2],
        quat_z: quat[3],
        stamp: Some(stamp(wire::SourceRole::PayloadDevice, 1)),
        yaw_frame: frame as i32,
        ..wire::GimbalAttitude::default()
    }
}

#[test]
fn a_truth_sample_becomes_the_pose_of_its_body_origin_on_its_clock() {
    let truth = truth_pose(&truth_sample(5_000, FRAC_PI_2)).expect("a pose");
    assert_eq!(truth.clock, MeasurementClock::Simulation);
    let pose = truth.pose;
    assert_eq!(pose.sim_time_ns, 5_000);
    assert_eq!(
        (pose.latitude_deg, pose.longitude_deg, pose.altitude_m),
        N51
    );
    assert_eq!(pose.vertical_datum, 2);
    let expected = yaw_pitch(FRAC_PI_2, 0.0).map(f64::from);
    let got = [pose.q_w, pose.q_x, pose.q_y, pose.q_z];
    for (got, expected) in got.iter().zip(expected) {
        assert!((got - expected).abs() < 1e-6, "{got:?} vs {expected:?}");
    }
    assert_eq!(pose.gimbal, None, "no gimbal reported, none sent");
}

#[test]
fn truth_a_camera_cannot_stand_on_yields_no_pose() {
    let refuse = |change: fn(&mut wire::TelemetrySample)| {
        let mut sample = truth_sample(1, 0.0);
        change(&mut sample);
        assert!(truth_pose(&sample).is_none());
    };
    refuse(|sample| sample.sim_truth = None);
    refuse(|sample| {
        let truth = sample.sim_truth.as_mut().expect("truth");
        truth.stamp = Some(stamp(wire::SourceRole::OperationalEstimate, 1));
    });
    refuse(|sample| sample.sim_truth.as_mut().expect("truth").valid_flags = 0b1011);
    refuse(|sample| sample.sim_truth.as_mut().expect("truth").valid_flags = 0b1110);
    refuse(|sample| sample.sim_truth.as_mut().expect("truth").geodetic = None);
    refuse(|sample| {
        let truth = sample.sim_truth.as_mut().expect("truth");
        truth.geodetic.as_mut().expect("fix").horizontal_datum = 0;
    });
    refuse(|sample| {
        let truth = sample.sim_truth.as_mut().expect("truth");
        truth.geodetic.as_mut().expect("fix").height_m = f64::NAN;
    });
    refuse(|sample| sample.sim_truth.as_mut().expect("truth").quat_w = 0.0);
    refuse(|sample| {
        let truth = sample.sim_truth.as_mut().expect("truth");
        let mut stamp = stamp(wire::SourceRole::SimulationTruth, 1);
        stamp.clock = wire::MeasurementClock::Unspecified as i32;
        truth.stamp = Some(stamp);
    });
}

#[test]
fn a_gimbal_pan_is_relative_to_the_body_heading_whatever_frame_it_reports() {
    let body_yaw = 1.0;
    let mut sample = truth_sample(1, body_yaw);
    sample.gimbal = Some(Box::new(gimbal(0.4, -0.6, wire::GimbalYawFrame::Vehicle)));
    let pointing = truth_pose(&sample)
        .expect("pose")
        .pose
        .gimbal
        .expect("gimbal");
    assert!((pointing.pan_rad - 0.4).abs() < 1e-5, "{pointing:?}");
    assert!((pointing.tilt_rad + 0.6).abs() < 1e-5, "{pointing:?}");
    assert_eq!(pointing.zoom_detent, 0);

    sample.gimbal = Some(Box::new(gimbal(
        body_yaw + 0.4,
        -0.6,
        wire::GimbalYawFrame::Earth,
    )));
    let pointing = truth_pose(&sample)
        .expect("pose")
        .pose
        .gimbal
        .expect("gimbal");
    assert!((pointing.pan_rad - 0.4).abs() < 1e-5, "{pointing:?}");

    sample.gimbal = Some(Box::new(gimbal(
        0.4,
        -0.6,
        wire::GimbalYawFrame::Unspecified,
    )));
    assert_eq!(truth_pose(&sample).expect("pose").pose.gimbal, None);
    let mut failed = gimbal(0.4, -0.6, wire::GimbalYawFrame::Vehicle);
    failed.failure_flags = 1;
    sample.gimbal = Some(Box::new(failed));
    assert_eq!(truth_pose(&sample).expect("pose").pose.gimbal, None);
}

async fn next_pose(producer: &mut TcpStream) -> pilotage_sim_video::wire::BridgeViewPose {
    let envelope = tokio::time::timeout(DEADLINE, read_envelope(producer))
        .await
        .expect("a pose before the deadline")
        .expect("read")
        .expect("an envelope");
    match envelope.payload {
        Some(Payload::ViewPose(pose)) => pose,
        other => panic!("expected a view pose, got {other:?}"),
    }
}

fn datagram(sample: wire::TelemetrySample) -> ToConnection {
    ToConnection::Datagram {
        class: DatagramClass::Telemetry,
        bytes: encode_telemetry_datagram(sample),
    }
}

#[test]
fn a_producer_gets_each_new_truth_pose_and_its_frames_carry_the_truth_clock() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let (engine_tx, mut engine_rx) = mpsc::channel(8);
        let (mut frames, task) = spawn(listener, &engine_tx);
        drop(engine_tx);
        let Some(ToEngine::ClientConnected { client, sender }) = engine_rx.recv().await else {
            panic!("the attachment registers with the engine");
        };
        assert_eq!(client, SIM_CAMERA_CLIENT);

        // Truth that arrives before the producer is sent when it attaches.
        sender
            .send(datagram(truth_sample(1_000, 0.0)))
            .await
            .expect("send");
        let mut producer = TcpStream::connect(address).await.expect("connect");
        assert_eq!(next_pose(&mut producer).await.sim_time_ns, 1_000);

        // The same observation re-broadcast is not a new pose; the next one is.
        sender
            .send(datagram(truth_sample(1_000, 0.0)))
            .await
            .expect("send");
        sender
            .send(datagram(truth_sample(2_000, 0.0)))
            .await
            .expect("send");
        assert_eq!(next_pose(&mut producer).await.sim_time_ns, 2_000);

        let frame = BridgeEnvelope {
            payload: Some(Payload::Frame(BridgeFrame {
                width: 2,
                height: 1,
                pixel_format: "RGB_INT8".to_owned(),
                sim_time_ns: 2_000,
                rgb: vec![1, 2, 3, 4, 5, 6],
                camera_id: 2,
                calibration_id: 9,
            })),
        };
        producer
            .write_all(&frame.encode_length_delimited_to_vec())
            .await
            .expect("write frame");
        let stamped = tokio::time::timeout(DEADLINE, frames.recv())
            .await
            .expect("a frame before the deadline")
            .expect("a frame");
        assert_eq!(stamped.source_id, 2);
        assert_eq!(stamped.capture.stamp.acquired_at_ns, 2_000);
        assert_eq!(stamped.capture.stamp.clock, MeasurementClock::Simulation);
        assert_eq!(
            stamped.capture.mapping,
            CaptureClockMapping::identity(MeasurementClock::Simulation)
        );
        assert_eq!(stamped.rgb, [1, 2, 3, 4, 5, 6]);
        assert_eq!(
            stamped.capture.calibration_id,
            pilotage_adapter_api::CalibrationId(9),
            "the producer's per-frame calibration reaches the stamp"
        );

        // The engine actor ending ends the attachment and closes the frames.
        drop(sender);
        tokio::time::timeout(DEADLINE, task)
            .await
            .expect("the attachment exits with the engine")
            .expect("join");
        assert!(frames.recv().await.is_none());
    });
}
