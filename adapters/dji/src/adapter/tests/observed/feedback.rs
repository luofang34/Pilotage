use pilotage_adapter_api::{Disposition, LinkLossPolicy, RejectReason, VehicleAdapter};

use super::{HOST_PITCH_RATE_LIMIT_RPS, TestResult, frame, pose, ready_at, recenter, targets};

#[test]
fn positive_feedback_stays_measured_while_manual_targets_stay_within_policy() -> TestResult {
    let mut harness = ready_at(1)?;
    let measured = harness.telemetry(20).samples.remove(0).payload.unwrap();
    assert!((measured.pitch.unwrap().angle_rad.unwrap().to_degrees() - 0.1).abs() < 0.00001);
    assert_eq!(
        harness.apply(frame(0.0), 20).disposition,
        Disposition::Accepted
    );
    harness.step(21);
    assert!(targets(&harness)?.is_empty());
    assert_eq!(
        harness
            .apply(frame(-HOST_PITCH_RATE_LIMIT_RPS), 30)
            .disposition,
        Disposition::Transformed
    );
    harness.step(31);
    assert_eq!(targets(&harness)?, [-299]);
    assert_eq!(
        harness
            .adapter
            .status_cached()
            .latest_angles
            .unwrap()
            .pitch_tenths,
        1
    );
    assert!(harness.adapter.profile.pitch_target(1).is_err());
    assert!(harness.adapter.profile.pitch_target(-901).is_err());
    Ok(())
}

#[test]
fn neutralize_from_positive_feedback_sends_a_bounded_target_without_faking_arrival() -> TestResult {
    let mut harness = ready_at(1)?;
    harness.link_loss(Some(LinkLossPolicy::Neutralize), 20)?;
    assert_eq!(targets(&harness)?, [0]);
    let state = harness.adapter.status_cached();
    assert!(state.control_locked);
    assert_eq!(state.latest_angles.unwrap().pitch_tenths, 1);
    let progress = harness
        .adapter
        .connection
        .execution
        .wire_progress()
        .unwrap();
    assert!(progress.sent_at.is_some());
    assert!(progress.observed_complete_at.is_none());
    harness.link_loss(None, 30)?;
    assert!(!harness.adapter.status_cached().control_locked);
    Ok(())
}

#[test]
fn recenter_above_the_command_ceiling_uses_raw_feedback_without_extending_the_ceiling() -> TestResult
{
    let mut harness = ready_at(1)?;
    assert_eq!(
        harness.apply(recenter(), 20).disposition,
        Disposition::Accepted
    );
    harness.step(21);
    assert_eq!(targets(&harness)?, [0]);
    assert_eq!(
        harness
            .adapter
            .status_cached()
            .latest_angles
            .unwrap()
            .pitch_tenths,
        1
    );
    harness.apply(frame(0.0), 40);
    harness.step(41);
    assert_eq!(targets(&harness)?, [0]);
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
fn the_configured_lower_target_bound_does_not_discard_valid_feedback() -> TestResult {
    let mut harness = super::Harness::new();
    harness.select_profile(turret_dji::Mini3ControlProfile::new(
        -50,
        turret_core::AngularRate::radians_per_second(HOST_PITCH_RATE_LIMIT_RPS)?,
    )?);
    for sequence in 0..3 {
        pose(&mut harness, -51, sequence, u64::from(sequence) * 10)?;
    }
    assert_eq!(harness.adapter.status_cached().consecutive_poses, 3);
    assert_eq!(
        harness.apply(frame(0.0), 20).disposition,
        Disposition::Accepted
    );
    harness.link_loss(Some(LinkLossPolicy::Neutralize), 30)?;
    assert_eq!(targets(&harness)?, [-50]);
    assert!(harness.adapter.profile.pitch_target(-51).is_err());
    let angle = harness
        .telemetry(30)
        .samples
        .remove(0)
        .payload
        .unwrap()
        .pitch
        .unwrap();
    assert!((angle.angle_rad.unwrap().to_degrees() + 5.1).abs() < 0.00001);
    Ok(())
}

#[test]
fn invalid_pitch_feedback_cannot_publish_angles_or_authorize_control() -> TestResult {
    let mut harness = ready_at(0)?;
    pose(&mut harness, i16::MAX, 3, 40)?;
    assert_eq!(harness.adapter.status_cached().consecutive_poses, 0);
    let before = harness.written()?;
    assert_eq!(
        harness.apply(frame(0.0), 40).disposition,
        Disposition::Rejected(RejectReason::MeasurementUnavailable)
    );
    harness.step(41);
    assert_eq!(harness.written()?, before);
    assert!(
        harness
            .telemetry(41)
            .samples
            .iter()
            .all(|sample| sample.payload.is_none())
    );
    Ok(())
}

#[test]
fn changed_feedback_cancels_an_unsafe_queued_target_at_the_write_boundary() -> TestResult {
    for partial in [false, true] {
        let mut harness = ready_at(1)?;
        let contract = harness.adapter.capabilities();
        harness.set_write_limit(if partial { 3 } else { 0 })?;
        assert_eq!(
            harness.apply(recenter(), 20).disposition,
            Disposition::Accepted
        );
        harness.step(21);
        assert!(harness.adapter.status_cached().pending_bytes > 0);
        let written = harness.written()?;
        harness.set_write_limit(usize::MAX)?;
        pose(&mut harness, 800, 3, 40)?;
        assert_eq!(harness.written()?, written);
        assert_eq!(harness.adapter.status_cached().pending_bytes, 0);
        assert!(harness.adapter.control.observed_write.is_none());
        assert_eq!(
            matches!(
                harness.adapter.status_cached().fault,
                Some(super::DjiFault::PartialCommand)
            ),
            partial
        );
        if !partial {
            assert_eq!(harness.adapter.capabilities(), contract);
            assert_eq!(
                harness
                    .adapter
                    .status_cached()
                    .latest_angles
                    .unwrap()
                    .pitch_tenths,
                800
            );
            pose(&mut harness, 1, 4, 50)?;
            harness.apply(frame(0.0), 50);
            harness.step(51);
            assert_eq!(harness.written()?, written);
        }
    }
    Ok(())
}
