use std::sync::atomic::Ordering;

use pilotage_adapter_api::{Disposition, VehicleAdapter};

use super::harness::{Harness, frame, observed_poses, packets};
use crate::logiclink::LogicLinkParser;

mod preemption;

#[cfg(test)]
fn commands(harness: &Harness) -> Vec<crate::DumlFrame> {
    let mut frames = Vec::new();
    LogicLinkParser::new()
        .push(&harness.written().unwrap(), |envelope| {
            frames.extend(packets(envelope.payload));
        })
        .unwrap();
    frames
}

#[test]
fn requests_coalesce_and_reserve_three_fresh_sequences_with_write_spacing() {
    let mut h = Harness::mobile();
    h.feed_model(87, 0).unwrap();
    let video = h.adapter.subscribe_encoded_video().unwrap();
    assert!(h.adapter.capabilities().execution.render_capable);
    video.keyframe_request.store(true, Ordering::Release);
    h.step(0);
    assert_eq!(commands(&h).len(), 2);
    assert_eq!(
        h.deadlines().unwrap()[1].duration_since(h.adapter.origin),
        std::time::Duration::from_millis(300)
    );
    h.step(49);
    assert_eq!(commands(&h).len(), 2);
    h.step(50);
    assert_eq!(commands(&h).len(), 3);
    h.step(100);
    let packets = commands(&h);
    assert_eq!(packets.len(), 4);
    assert_eq!(
        packets[1..]
            .iter()
            .map(crate::DumlFrame::sequence)
            .collect::<Vec<_>>(),
        vec![2, 3, 4]
    );
    assert_eq!(h.recorded("tx").unwrap(), h.written().unwrap());
    h.feed_model(87, 1).unwrap();
    h.step(999);
    assert_eq!(commands(&h).len(), 4);
    video.keyframe_request.store(true, Ordering::Release);
    video.keyframe_request.store(true, Ordering::Release);
    h.step(1000);
    assert_eq!(commands(&h).last().unwrap().sequence(), 5);
    h.step(1050);
    h.step(1100);
    h.step(3000);
    assert_eq!(commands(&h).len(), 7);
}

#[test]
fn partial_keyframe_expires_without_replaying_or_interleaving_pitch() {
    let mut h = Harness::mobile();
    h.feed_model(87, 0).unwrap();
    for (index, pose) in observed_poses().iter().enumerate() {
        h.feed(&crate::logiclink::encode_control(&packets(pose)[0]))
            .unwrap();
        h.step(index as u64 * 10);
    }
    let video = h.adapter.subscribe_encoded_video().unwrap();
    h.set_write_limit(4).unwrap();
    video.keyframe_request.store(true, Ordering::Release);
    h.step(30);
    let prefix = h.written().unwrap();
    assert!(matches!(
        h.apply(frame(-0.01), 40).disposition,
        Disposition::Rejected(_)
    ));
    h.set_write_limit(usize::MAX).unwrap();
    h.step(530);
    assert!(h.adapter.status_cached().fault.is_some());
    assert_eq!(h.written().unwrap(), prefix);
    video.keyframe_request.store(true, Ordering::Release);
    h.step(1000);
    assert_eq!(h.written().unwrap(), prefix);
}

#[test]
fn write_completion_time_controls_spacing() {
    let mut h = Harness::mobile();
    h.feed_model(87, 0).unwrap();
    h.step(0);
    let video = h.adapter.subscribe_encoded_video().unwrap();
    video.keyframe_request.store(true, Ordering::Release);
    let mut calls = 0_usize;
    h.adapter.step_with_time_blocking(|| {
        calls = calls.wrapping_add(1);
        std::time::Duration::from_millis(if calls < 5 { 10 } else { 70 })
    });
    assert_eq!(commands(&h).len(), 2);
    h.step(119);
    assert_eq!(commands(&h).len(), 2);
    h.step(120);
    assert_eq!(commands(&h).len(), 3);
    h.step(170);
    assert_eq!(commands(&h).len(), 4);
}

#[test]
fn write_completed_after_deadline_keeps_evidence_and_locks_the_connection() {
    let mut h = Harness::mobile();
    h.feed_model(87, 0).unwrap();
    h.step(0);
    let video = h.adapter.subscribe_encoded_video().unwrap();
    video.keyframe_request.store(true, Ordering::Release);
    let mut calls = 0_usize;
    h.adapter.step_with_time_blocking(|| {
        calls = calls.wrapping_add(1);
        std::time::Duration::from_millis(if calls < 5 { 10 } else { 510 })
    });
    assert!(h.adapter.status_cached().fault.is_some());
    let written = h.written().unwrap();
    assert_eq!(commands(&h).len(), 2);
    assert_eq!(h.recorded("tx").unwrap(), written);
    assert_eq!(
        h.adapter.status_cached().written_bytes,
        written.len() as u64
    );
    h.step(1000);
    assert_eq!(h.written().unwrap(), written);
}

#[test]
fn serial_has_no_video_recovery_control_surface() {
    let mut h = Harness::new();
    assert!(h.adapter.subscribe_encoded_video().is_none());
    h.step(0);
    h.step(1000);
    assert_eq!(packets(&h.written().unwrap()).len(), 1);
}
