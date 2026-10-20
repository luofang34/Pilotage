use super::harness::{Harness, frame, observed_poses, packets};
use crate::logiclink::{CONTROL_CHANNEL, LogicLinkParser};
use pilotage_adapter_api::{AvionicsAttitudeSample, Disposition, VehicleAdapter};
use std::time::Duration;

#[test]
fn upper_capture_reaches_real_payload_state_without_faking_vehicle_pose() {
    let mut harness = Harness::mobile();
    let mut video = harness.adapter.subscribe_encoded_video().unwrap();
    harness
        .feed(include_bytes!(
            "../../logiclink/fixtures/mini3-upper-receive.bin"
        ))
        .unwrap();
    harness.step(10);
    assert!(harness.adapter.status_cached().fault.is_none());
    let samples = harness.telemetry(10).samples;
    assert_eq!(samples.len(), 1);
    assert!(matches!(
        samples[0].avionics.and_then(|avionics| avionics.attitude),
        Some(AvionicsAttitudeSample::ReportedEuler { .. })
    ));
    assert!(samples[0].pose.is_none());
    assert!(samples[0].speed.is_none());
    let state = samples[0].payload.as_ref().unwrap();
    let yaw = state.yaw.as_ref().unwrap();
    assert!((yaw.angle_rad.unwrap() - 66.1_f32.to_radians()).abs() < 0.001);
    assert!(yaw.sampled_at.is_none());
    assert_eq!(
        yaw.frame,
        pilotage_protocol::wire::PayloadReferenceFrame::Unspecified as i32
    );
    assert_eq!(video.receiver.try_recv().unwrap().bytes.len(), 2040);
    assert_eq!(
        video.receiver.try_recv().unwrap().received_at,
        Duration::from_millis(10)
    );
    assert!(video.receiver.try_recv().is_err());
    assert_eq!(harness.adapter.video_sources().len(), 1);
    assert!(harness.adapter.subscribe_encoded_video().is_none());
}

#[test]
fn mobile_control_uses_the_same_admission_and_actual_write_evidence() {
    let mut harness = Harness::mobile();
    harness.feed_model(87, 0).unwrap();
    for (index, pose) in observed_poses().iter().enumerate() {
        let command = packets(pose).remove(0);
        harness
            .feed(&crate::logiclink::encode_control(&command))
            .unwrap();
        harness.step(index as u64 * 10);
    }
    assert_eq!(
        harness.apply(frame(-5_f32.to_radians()), 20).disposition,
        Disposition::Transformed
    );
    harness.set_write_limit(4).unwrap();
    harness.step(30);
    assert!(harness.adapter.status_cached().pending_bytes > 0);
    harness.set_write_limit(usize::MAX).unwrap();
    harness.step(40);
    assert_eq!(harness.adapter.status_cached().pending_bytes, 0);
    let deadlines = harness.deadlines().unwrap();
    assert_eq!(deadlines[1], deadlines[2]);
    assert_eq!(
        deadlines[1].duration_since(harness.adapter.origin),
        Duration::from_millis(120)
    );
    let bytes = harness.written().unwrap();
    assert_eq!(harness.recorded("tx").unwrap(), bytes);
    let mut parser = LogicLinkParser::new();
    let mut commands = Vec::new();
    parser
        .push(&bytes, |envelope| {
            assert_eq!(envelope.channel, CONTROL_CHANNEL);
            commands.extend(packets(envelope.payload));
        })
        .unwrap();
    parser.finish().unwrap();
    assert_eq!(commands.len(), 2);
    assert_eq!(
        (commands[1].command_set(), commands[1].command_id()),
        (4, 0x14)
    );
    let payload = commands[1].payload();
    assert_eq!(i16::from_le_bytes([payload[4], payload[5]]), -99);
}

#[test]
fn damaged_mobile_envelope_closes_control_before_a_queued_write() {
    let mut harness = Harness::mobile();
    harness.feed(&[0x55, 0xcd, 0, 0, 0, 0, 0, 0]).unwrap();
    harness.step(0);
    assert!(harness.adapter.status_cached().fault.is_some());
    assert!(harness.written().unwrap().is_empty());
    assert!(matches!(
        harness.apply(frame(0.0), 0).disposition,
        Disposition::Rejected(_)
    ));
}
