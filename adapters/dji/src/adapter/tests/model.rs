use std::sync::atomic::Ordering;
use std::time::Duration;

use pilotage_adapter_api::{Disposition, RejectReason};
use pilotage_protocol::ControlAction;
use turret_core::Capability;

use super::{
    TestResult,
    harness::{Harness, frame, observed_poses, packets, seal_duml},
};
use crate::adapter::DjiFault;

fn mobile_commands(harness: &Harness) -> Vec<crate::DumlFrame> {
    let mut frames = Vec::new();
    crate::logiclink::LogicLinkParser::new()
        .push(&harness.written().unwrap(), |envelope| {
            frames.extend(packets(envelope.payload));
        })
        .unwrap();
    frames
}

fn feed_pose(harness: &mut Harness, sequence: u16) -> TestResult {
    let mut bytes = observed_poses().remove(0);
    bytes[6..8].copy_from_slice(&sequence.to_le_bytes());
    seal_duml(&mut bytes);
    harness.feed(&bytes)?;
    Ok(())
}

#[test]
fn gimbal_feedback_without_a_confirmed_model_cannot_authorize_writes() -> TestResult {
    for model in [None, Some(0), Some(86), Some(88), Some(255)] {
        let mut harness = Harness::mobile();
        if let Some(model) = model {
            harness.feed_model(model, 0)?;
        }
        for (index, pose) in observed_poses().iter().enumerate() {
            harness.feed(&crate::logiclink::encode_control(&packets(pose)[0]))?;
            harness.step(index as u64 * 10);
        }
        assert_eq!(harness.adapter.status_cached().consecutive_poses, 3);
        assert_eq!(
            harness.apply(frame(-0.01), 20).disposition,
            Disposition::Rejected(RejectReason::MeasurementUnavailable)
        );
        let video = harness.adapter.subscribe_encoded_video().unwrap();
        video.keyframe_request.store(true, Ordering::Release);
        harness.step(30);
        assert_eq!(mobile_commands(&harness).len(), 1);
        assert!(
            harness
                .telemetry(30)
                .samples
                .iter()
                .all(|sample| { sample.avionics.is_none() && sample.fc_state.is_none() })
        );
        assert!(matches!(
            harness
                .adapter
                .connection
                .execution
                .capabilities()
                .gimbal
                .axes
                .pitch
                .position,
            Capability::Unavailable { .. }
        ));
    }
    Ok(())
}

#[test]
fn initial_model_confirmation_requires_new_gimbal_arrivals() -> TestResult {
    let mut harness = Harness::new();
    harness.feed_model(88, 0)?;
    for (sequence, millis) in [(0, 0), (1, 10), (2, 20)] {
        feed_pose(&mut harness, sequence)?;
        harness.step(millis);
    }
    harness.feed_model(87, 1)?;
    harness.step(30);
    assert_eq!(harness.adapter.status_cached().consecutive_poses, 0);
    assert!(matches!(
        harness.apply(frame(-0.01), 30).disposition,
        Disposition::Rejected(_)
    ));
    for (sequence, millis) in [(3, 40), (4, 50), (5, 60)] {
        feed_pose(&mut harness, sequence)?;
        harness.step(millis);
    }
    assert_eq!(
        harness.apply(frame(-0.01), 60).disposition,
        Disposition::Transformed
    );
    harness.step(61);
    assert_eq!(packets(&harness.written()?).len(), 2);
    Ok(())
}

#[test]
fn a_different_fc_type_revokes_airframe_without_replay_renewal() -> TestResult {
    let mut harness = Harness::new();
    harness.feed_model(87, 10)?;
    harness.step(0);
    assert!(harness.telemetry(0).samples[0].fc_state.is_some());
    harness.feed_model(88, 11)?;
    harness.step(100);
    assert!(harness.telemetry(100).samples.is_empty());
    assert!(
        harness
            .adapter
            .observations
            .model_deadline(Duration::from_millis(100))
            .is_none()
    );
    for sequence in [10, 11] {
        harness.feed_model(87, sequence)?;
        harness.step(200);
        assert!(harness.telemetry(200).samples.is_empty());
    }
    harness.feed_model(87, 12)?;
    harness.step(250);
    let batch = harness.telemetry(250);
    let motor = batch.samples[0].fc_state.unwrap().motor_state.unwrap();
    assert_eq!(motor.stamp.acquired_at_ns, 250_000_000);
    assert_eq!(motor.stamp.sequence, 2);
    Ok(())
}

#[test]
fn a_different_model_with_a_restarted_sequence_still_revokes_queued_control() -> TestResult {
    let mut harness = Harness::new();
    harness.prime()?;
    harness.feed_model(87, 100)?;
    harness.step(20);
    assert_eq!(
        harness.apply(frame(-0.01), 20).disposition,
        Disposition::Transformed
    );
    let before = harness.written()?;
    harness.feed_model(88, 0)?;
    harness.step(30);
    assert_eq!(harness.written()?, before);
    assert!(harness.adapter.connection.pending_pitch().is_none());
    assert!(
        harness
            .telemetry(30)
            .samples
            .iter()
            .all(|sample| sample.fc_state.is_none())
    );
    harness.feed_model(87, 100)?;
    harness.step(40);
    assert!(
        harness
            .adapter
            .observations
            .model_deadline(Duration::from_millis(40))
            .is_none()
    );
    Ok(())
}

#[test]
fn fresh_gimbal_reports_cannot_extend_the_model_binding() -> TestResult {
    let mut harness = Harness::new();
    harness.prime()?;
    for (sequence, millis) in [(20_000, 200), (20_001, 290), (20_002, 300)] {
        feed_pose(&mut harness, sequence)?;
        harness.step(millis);
    }
    assert_eq!(
        harness
            .adapter
            .status_cached()
            .latest_angles
            .unwrap()
            .sequence,
        20_002
    );
    assert_eq!(
        harness.apply(frame(-0.01), 300).disposition,
        Disposition::Rejected(RejectReason::MeasurementUnavailable)
    );
    assert!(
        harness
            .telemetry(300)
            .samples
            .iter()
            .all(|sample| sample.fc_state.is_none())
    );
    Ok(())
}

#[test]
fn changed_model_and_same_batch_recovery_cannot_resurrect_a_queued_recenter() -> TestResult {
    let mut harness = Harness::new();
    harness.prime()?;
    let mut recenter = frame(0.0);
    recenter.actions.push(ControlAction::GimbalRecenter);
    assert_eq!(
        harness.apply(recenter, 20).disposition,
        Disposition::Accepted
    );
    let before = harness.written()?;
    harness.feed_model(88, 1)?;
    harness.feed_model(87, 2)?;
    harness.step(30);
    assert_eq!(harness.written()?, before);
    assert!(harness.adapter.connection.pending_pitch().is_none());
    for (sequence, millis) in [(20_000, 40), (20_001, 50), (20_002, 60)] {
        feed_pose(&mut harness, sequence)?;
        harness.step(millis);
    }
    assert_eq!(
        harness.apply(frame(0.0), 60).disposition,
        Disposition::Accepted
    );
    harness.step(61);
    assert_eq!(harness.written()?, before);
    assert!(harness.adapter.status_cached().fault.is_none());
    Ok(())
}

#[test]
fn expired_model_reconfirmed_during_read_cancels_old_recenter() -> TestResult {
    let mut harness = Harness::new();
    harness.prime()?;
    feed_pose(&mut harness, 20_000)?;
    harness.step(250);
    let mut recenter = frame(0.0);
    recenter.actions.push(ControlAction::GimbalRecenter);
    assert_eq!(
        harness.apply(recenter, 250).disposition,
        Disposition::Accepted
    );
    let before = harness.written()?;
    harness.feed_model(87, 1)?;
    feed_pose(&mut harness, 20_001)?;
    harness.step(300);
    assert_eq!(harness.written()?, before);
    assert!(harness.adapter.connection.pending_pitch().is_none());
    for (sequence, millis) in [(20_002, 310), (20_003, 320)] {
        feed_pose(&mut harness, sequence)?;
        harness.step(millis);
    }
    assert_eq!(
        harness.apply(frame(0.0), 320).disposition,
        Disposition::Accepted
    );
    harness.step(321);
    assert_eq!(harness.written()?, before);
    Ok(())
}

#[test]
fn a_changed_model_closes_a_partial_pitch_without_writing_its_tail() -> TestResult {
    let mut harness = Harness::new();
    harness.prime()?;
    harness.apply(frame(-0.01), 20);
    harness.set_write_limit(3)?;
    harness.step(21);
    let before = harness.written()?;
    harness.feed_model(88, 1)?;
    harness.set_write_limit(usize::MAX)?;
    harness.step(30);
    assert!(matches!(
        harness.adapter.status_cached().fault,
        Some(DjiFault::PartialCommand)
    ));
    assert_eq!(harness.written()?, before);
    Ok(())
}

#[test]
fn keyframe_model_change_discards_the_old_request_even_if_identity_recovers() -> TestResult {
    for write_limit in [4, usize::MAX] {
        let mut harness = Harness::mobile();
        harness.feed_model(87, 0)?;
        harness.step(0);
        let video = harness.adapter.subscribe_encoded_video().unwrap();
        harness.set_write_limit(write_limit)?;
        video.keyframe_request.store(true, Ordering::Release);
        harness.step(10);
        let before = harness.written()?;
        harness.feed_model(88, 1)?;
        harness.feed_model(87, 2)?;
        harness.set_write_limit(usize::MAX)?;
        harness.step(60);
        harness.step(110);
        assert_eq!(harness.written()?, before);
        assert_eq!(
            harness.adapter.status_cached().fault.is_some(),
            write_limit == 4
        );
    }
    Ok(())
}

#[test]
fn keyframe_transport_deadline_and_waiting_packets_end_with_model_freshness() -> TestResult {
    let mut harness = Harness::mobile();
    harness.feed_model(87, 0)?;
    harness.step(0);
    let video = harness.adapter.subscribe_encoded_video().unwrap();
    video.keyframe_request.store(true, Ordering::Release);
    harness.step(250);
    assert_eq!(mobile_commands(&harness).len(), 2);
    assert_eq!(
        harness.deadlines()?[1].duration_since(harness.adapter.origin),
        Duration::from_millis(300)
    );
    let before = harness.written()?;
    harness.step(300);
    assert_eq!(harness.written()?, before);
    assert!(harness.adapter.status_cached().fault.is_none());
    Ok(())
}

#[test]
fn queued_pitch_transport_deadline_is_bounded_by_model_freshness() -> TestResult {
    let mut harness = Harness::new();
    harness.prime()?;
    feed_pose(&mut harness, 20_000)?;
    harness.step(250);
    assert_eq!(
        harness.apply(frame(-0.01), 250).disposition,
        Disposition::Transformed
    );
    harness.step(260);
    assert_eq!(
        harness.deadlines()?[1].duration_since(harness.adapter.origin),
        Duration::from_millis(300)
    );
    Ok(())
}

#[test]
fn continuous_model_reports_allow_keyframe_packets_without_extending_its_total_budget() -> TestResult
{
    let mut harness = Harness::mobile();
    harness.feed_model(87, 0)?;
    harness.step(0);
    let video = harness.adapter.subscribe_encoded_video().unwrap();
    video.keyframe_request.store(true, Ordering::Release);
    harness.step(250);
    harness.feed_model(87, 1)?;
    harness.step(290);
    harness.step(300);
    harness.step(350);
    assert_eq!(mobile_commands(&harness).len(), 4);
    let deadlines: Vec<_> = harness
        .deadlines()?
        .into_iter()
        .map(|deadline| deadline.duration_since(harness.adapter.origin))
        .collect();
    assert_eq!(
        deadlines[1..],
        [
            Duration::from_millis(300),
            Duration::from_millis(590),
            Duration::from_millis(590),
        ]
    );
    assert!(harness.adapter.status_cached().fault.is_none());
    Ok(())
}

#[test]
fn keyframe_write_receipt_after_model_expiry_preserves_bytes_and_closes() -> TestResult {
    let mut harness = Harness::mobile();
    harness.feed_model(87, 0)?;
    harness.step(0);
    let video = harness.adapter.subscribe_encoded_video().unwrap();
    video.keyframe_request.store(true, Ordering::Release);
    let mut calls = 0_usize;
    harness.adapter.step_with_time_blocking(|| {
        calls = calls.wrapping_add(1);
        Duration::from_millis(if calls < 5 { 250 } else { 350 })
    });
    let before = harness.written()?;
    assert_eq!(mobile_commands(&harness).len(), 2);
    assert_eq!(before, harness.recorded("tx")?);
    assert!(matches!(
        harness.adapter.status_cached().fault,
        Some(DjiFault::Keyframe(
            turret_dji::KeyframeAbort::DeadlineExpired
        ))
    ));
    harness.step(400);
    assert_eq!(harness.written()?, before);
    Ok(())
}
