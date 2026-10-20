use std::time::Duration;

use pilotage_adapter_api::{Disposition, LinkLossPolicy, RejectReason};
use pilotage_protocol::{
    ControlAction, ControlIntent, Generation, GimbalRateIntent, ScopeId, VehicleId,
};

use super::{
    TestResult,
    harness::{Harness, frame, observed_poses, packets},
};
use crate::adapter::{DjiFault, HOST_PITCH_RATE_LIMIT_RPS};

fn pose(harness: &mut Harness, pitch: i16, sequence: u16, millis: u64) -> TestResult {
    let mut bytes = observed_poses().remove(0);
    bytes[6..8].copy_from_slice(&sequence.to_le_bytes());
    bytes[11..13].copy_from_slice(&pitch.to_le_bytes());
    let end = bytes.len() - 2;
    let crc = turret_dji::frame_checksum(&bytes[..end]).to_le_bytes();
    bytes[end..].copy_from_slice(&crc);
    harness.feed(&bytes)?;
    harness.step(millis);
    Ok(())
}

fn ready_at(pitch: i16) -> Result<Harness, Box<dyn std::error::Error>> {
    let mut harness = Harness::new();
    for sequence in 0..3 {
        pose(&mut harness, pitch, sequence, u64::from(sequence) * 10)?;
    }
    Ok(harness)
}

fn recenter() -> pilotage_protocol::ScopedControlFrame {
    let mut request = frame(0.0);
    request.actions.push(ControlAction::GimbalRecenter);
    request
}

fn targets(harness: &Harness) -> Result<Vec<i16>, Box<dyn std::error::Error>> {
    Ok(packets(&harness.written()?)
        .into_iter()
        .filter(|packet| (packet.command_set(), packet.command_id()) == (4, 0x14))
        .map(|packet| i16::from_le_bytes([packet.payload()[4], packet.payload()[5]]))
        .collect())
}

#[test]
fn full_writes_deduplicate_unchanged_targets_but_new_feedback_advances_them() -> TestResult {
    let mut harness = ready_at(0)?;
    harness.apply(frame(-HOST_PITCH_RATE_LIMIT_RPS), 20);
    harness.step(21);
    for millis in [40, 80, 120] {
        harness.apply(frame(-HOST_PITCH_RATE_LIMIT_RPS), millis);
        harness.step(millis);
    }
    assert_eq!(targets(&harness)?, vec![-50]);
    pose(&mut harness, -10, 3, 130)?;
    harness.apply(frame(-HOST_PITCH_RATE_LIMIT_RPS), 140);
    harness.step(141);
    assert_eq!(targets(&harness)?, vec![-50, -60]);
    Ok(())
}

#[test]
fn one_recenter_follows_fresh_feedback_from_full_down_to_horizontal() -> TestResult {
    let mut harness = ready_at(-900)?;
    harness.apply(recenter(), 20);
    harness.step(21);
    for step in 1..=180_u16 {
        let now = 20 + u64::from(step) * 100;
        pose(&mut harness, -900 + step as i16 * 5, step + 2, now)?;
        harness.apply(frame(0.0), now);
        harness.step(now + 1);
    }
    let targets = targets(&harness)?;
    assert_eq!(targets.first(), Some(&-850));
    assert_eq!(targets.last(), Some(&0));
    assert_eq!(targets.iter().filter(|target| **target == 0).count(), 1);
    assert!(targets.windows(2).all(|pair| pair[1] - pair[0] == 5));
    assert_eq!(
        harness
            .adapter
            .status_cached()
            .latest_angles
            .unwrap()
            .pitch_tenths,
        0
    );
    let progress = harness
        .adapter
        .connection
        .execution
        .wire_progress()
        .unwrap();
    assert!(progress.observed_complete_at.is_none());
    Ok(())
}

#[test]
fn an_unchanged_pending_target_keeps_its_original_write_deadline() -> TestResult {
    let mut harness = ready_at(0)?;
    harness.set_write_limit(0)?;
    harness.apply(frame(-HOST_PITCH_RATE_LIMIT_RPS), 20);
    let identity = harness.adapter.connection.pending_pitch_identity();
    harness.apply(frame(-HOST_PITCH_RATE_LIMIT_RPS), 80);
    assert_eq!(
        harness.adapter.connection.pending_pitch_identity(),
        identity
    );
    harness.set_write_limit(usize::MAX)?;
    harness.step(120);
    assert!(targets(&harness)?.is_empty());
    assert_eq!(harness.adapter.status_cached().pending_bytes, 0);
    Ok(())
}

#[test]
fn an_expired_pending_recenter_cannot_resume_with_continuous_neutral_input() -> TestResult {
    let mut harness = ready_at(-300)?;
    harness.set_write_limit(0)?;
    harness.apply(recenter(), 20);
    harness.apply(frame(0.0), 80);
    harness.step(120);
    harness.set_write_limit(usize::MAX)?;
    pose(&mut harness, -295, 3, 130)?;
    harness.apply(frame(0.0), 140);
    harness.step(141);
    assert!(targets(&harness)?.is_empty());
    assert_eq!(harness.adapter.status_cached().pending_bytes, 0);
    Ok(())
}

#[test]
fn expired_or_rejected_input_cancels_recenter_without_neutral_resumption() -> TestResult {
    for reject in [false, true] {
        let mut harness = ready_at(-300)?;
        harness.apply(recenter(), 20);
        harness.step(21);
        if reject {
            let mut expired = frame(0.0);
            expired.admitted_timing.as_mut().unwrap().send_remaining = Duration::ZERO;
            assert!(matches!(
                harness.apply(expired, 40).disposition,
                Disposition::Rejected(_)
            ));
        }
        let now = if reject { 80 } else { 121 };
        pose(&mut harness, -295, 3, now)?;
        harness.apply(frame(0.0), now);
        harness.step(now + 1);
        assert_eq!(targets(&harness)?, vec![-250]);
    }
    Ok(())
}

#[test]
fn rejected_demand_from_current_authority_cancels_an_active_recenter() -> TestResult {
    let mut yaw = frame(0.0);
    yaw.intent = Some(ControlIntent::GimbalRate(GimbalRateIntent {
        pitch_rate: 0.0,
        yaw_rate: 0.01,
    }));
    let mut arm = frame(0.0);
    arm.actions.push(ControlAction::Arm);
    for rejected in [
        frame(f32::NAN),
        frame(HOST_PITCH_RATE_LIMIT_RPS * 2.0),
        yaw,
        arm,
    ] {
        let mut harness = ready_at(-300)?;
        harness.apply(recenter(), 20);
        harness.step(21);
        assert!(matches!(
            harness.apply(rejected, 40).disposition,
            Disposition::Rejected(_)
        ));
        pose(&mut harness, -295, 3, 80)?;
        harness.apply(frame(0.0), 80);
        harness.step(81);
        assert_eq!(targets(&harness)?, vec![-250]);
    }
    Ok(())
}

#[test]
fn rejected_foreign_scope_or_authority_cannot_cancel_the_current_recenter() -> TestResult {
    let mut wrong_vehicle = frame(f32::NAN);
    wrong_vehicle.vehicle = VehicleId::new(2);
    wrong_vehicle.generation = Generation::new(2);
    let mut wrong_scope = frame(f32::NAN);
    wrong_scope.scope = ScopeId::new("vehicle.motion");
    wrong_scope.generation = Generation::new(2);
    let mut old_deadline = frame(0.0);
    old_deadline
        .admitted_timing
        .as_mut()
        .unwrap()
        .send_remaining = Duration::ZERO;
    let mut unbound_deadline = old_deadline.clone();
    unbound_deadline.generation = Generation::new(3);
    for rejected in [
        wrong_vehicle,
        wrong_scope,
        frame(f32::NAN),
        old_deadline,
        unbound_deadline,
    ] {
        let mut harness = ready_at(-300)?;
        let mut current = recenter();
        current.generation = Generation::new(2);
        harness.apply(current, 20);
        harness.step(21);
        assert!(matches!(
            harness.apply(rejected, 40).disposition,
            Disposition::Rejected(_)
        ));
        pose(&mut harness, -295, 3, 80)?;
        let mut neutral = frame(0.0);
        neutral.generation = Generation::new(2);
        harness.apply(neutral, 80);
        harness.step(81);
        assert_eq!(targets(&harness)?, vec![-250, -245]);
    }
    Ok(())
}
#[test]
fn feedback_loss_cancels_recenter_before_fresh_reports_return() -> TestResult {
    let mut harness = ready_at(-300)?;
    harness.apply(recenter(), 20);
    harness.step(21);
    harness.step(321);
    for sequence in 3..6 {
        pose(&mut harness, -295, sequence, 330 + u64::from(sequence) * 10)?;
    }
    harness.apply(frame(0.0), 390);
    harness.step(391);
    assert_eq!(targets(&harness)?, vec![-250]);
    Ok(())
}

#[test]
fn an_out_of_range_observation_cancels_recenter_before_valid_feedback_returns() -> TestResult {
    let mut harness = ready_at(-300)?;
    harness.apply(recenter(), 20);
    harness.step(21);
    pose(&mut harness, 1, 3, 40)?;
    for sequence in 4..7 {
        pose(&mut harness, -295, sequence, 10 + u64::from(sequence) * 10)?;
    }
    harness.apply(frame(0.0), 80);
    harness.step(81);
    assert_eq!(targets(&harness)?, vec![-250]);
    Ok(())
}

#[test]
fn new_authority_and_link_loss_require_a_new_recenter_request() -> TestResult {
    for link_loss in [false, true] {
        let mut harness = ready_at(-300)?;
        harness.apply(recenter(), 20);
        harness.step(21);
        if link_loss {
            harness.link_loss(Some(LinkLossPolicy::Neutralize), 30)?;
            harness.link_loss(None, 31)?;
        }
        let written = harness.written()?;
        let mut neutral = frame(0.0);
        neutral.generation = Generation::new(2);
        harness.apply(neutral, 40);
        harness.step(41);
        assert_eq!(harness.written()?, written);
    }
    Ok(())
}

#[test]
fn a_late_control_frame_does_not_cancel_a_partial_link_loss_hold() -> TestResult {
    let mut harness = ready_at(-300)?;
    harness.apply(recenter(), 20);
    harness.step(21);
    harness.set_write_limit(3)?;
    assert!(
        harness
            .link_loss(Some(LinkLossPolicy::Neutralize), 30)
            .is_err()
    );
    assert_eq!(harness.adapter.status_cached().pending_bytes, 18);
    assert_eq!(
        harness.apply(frame(0.0), 40).disposition,
        Disposition::Rejected(RejectReason::LinkLossEngaged)
    );
    assert_eq!(harness.adapter.status_cached().pending_bytes, 18);
    assert!(harness.adapter.status_cached().fault.is_none());
    harness.set_write_limit(usize::MAX)?;
    harness.step(50);
    assert_eq!(targets(&harness)?, vec![-250, -300]);
    assert!(harness.adapter.status_cached().fault.is_none());
    Ok(())
}

#[test]
fn a_new_recenter_after_manual_camera_motion_can_send_zero_again() -> TestResult {
    let mut harness = ready_at(-40)?;
    harness.apply(recenter(), 20);
    harness.step(21);
    pose(&mut harness, 0, 3, 40)?;
    harness.apply(frame(0.0), 40);
    pose(&mut harness, -40, 4, 80)?;
    harness.apply(recenter(), 80);
    harness.step(81);
    assert_eq!(targets(&harness)?, vec![0, 0]);
    Ok(())
}

#[test]
fn a_cancelled_partial_write_never_commits_a_plan_or_sends_a_successor() -> TestResult {
    let mut harness = ready_at(-300)?;
    harness.set_write_limit(3)?;
    harness.apply(recenter(), 20);
    harness.step(21);
    assert_eq!(harness.adapter.status_cached().pending_bytes, 18);
    let partial = harness.written()?;
    harness.apply(frame(-HOST_PITCH_RATE_LIMIT_RPS), 40);
    assert!(matches!(
        harness.adapter.status_cached().fault,
        Some(DjiFault::PartialCommand)
    ));
    assert!(harness.adapter.control.observed_write.is_none());
    harness.set_write_limit(usize::MAX)?;
    harness.step(41);
    assert_eq!(harness.written()?, partial);
    Ok(())
}

#[test]
fn capture_failure_after_a_full_write_cannot_commit_planner_history() -> TestResult {
    let mut harness = ready_at(-300)?;
    harness.apply(recenter(), 20);
    harness.fail_capture()?;
    harness.step(21);
    assert!(matches!(
        harness.adapter.status_cached().fault,
        Some(DjiFault::Recording { .. })
    ));
    assert!(harness.adapter.control.observed_write.is_none());
    assert!(
        harness
            .adapter
            .control
            .observed
            .as_mut()
            .unwrap()
            .plan(-300, 0.0, false, Duration::from_millis(40))?
            .is_none()
    );
    let progress = harness
        .adapter
        .connection
        .execution
        .wire_progress()
        .unwrap();
    assert!(progress.sent_at.is_some());
    assert!(progress.observed_complete_at.is_none());
    Ok(())
}

#[test]
fn a_missing_full_write_receipt_keeps_unknown_outcome_and_cancels_recenter() -> TestResult {
    let mut harness = ready_at(-300)?;
    harness.apply(recenter(), 20);
    harness.lose_write_receipt()?;
    harness.step(21);
    assert_eq!(targets(&harness)?, vec![-250]);
    assert!(harness.adapter.status_cached().control_locked);
    assert!(harness.adapter.control.observed_write.is_none());
    let progress = harness
        .adapter
        .connection
        .execution
        .wire_progress()
        .unwrap();
    assert!(progress.sent_at.is_none());
    assert_eq!(
        progress.stage,
        pilotage_protocol::wire::PayloadCommandStage::OutcomeUnknown as i32
    );
    let before = harness.written()?;
    harness.apply(frame(0.0), 40);
    harness.step(41);
    assert_eq!(harness.written()?, before);
    Ok(())
}
