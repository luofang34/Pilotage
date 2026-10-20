use pilotage_adapter_api::{Disposition, RejectReason, VehicleAdapter};
use pilotage_protocol::wire;
use pilotage_session::SessionAction;

use super::{TestResult, harness::packets};
use crate::adapter::HOST_PITCH_RATE_LIMIT_RPS;

mod session;
use session::RecoverySession;

#[test]
fn a_horizontal_measurement_overshoot_keeps_the_lease_and_raw_feedback() -> TestResult {
    let mut test = RecoverySession::ready()?;
    let generation = test.lease(22)?;
    let initial = test.frame(generation, 1, 0.0, 23);
    assert_eq!(test.enact(initial, 23)?, [Disposition::Accepted]);
    let contract = test.io.adapter.capabilities();
    for (offset, pitch, at) in [(1, 0, 40), (2, 1, 50), (3, 0, 60)] {
        test.pose_with_pitch(offset, Some(pitch), at)?;
        assert_eq!(test.io.adapter.capabilities(), contract);
        assert!(test.refresh(at).actions.is_empty());
        assert_eq!(test.lease(at)?, generation);
        let sample = test.io.telemetry(at).samples.remove(0);
        let angle = sample.payload.unwrap().pitch.unwrap().angle_rad.unwrap();
        assert!((angle.to_degrees() - f32::from(pitch) / 10.0).abs() < 0.00001);
        let neutral = test.frame(generation, u32::from(offset) + 1, 0.0, at + 1);
        assert_eq!(test.enact(neutral, at + 1)?, [Disposition::Accepted]);
    }
    let released = test.release(70);
    test.enact(released, 70)?;
    assert!(test.io.adapter.status_cached().control_locked);
    Ok(())
}

#[test]
fn release_and_neutral_recovery_keep_a_usable_lease_and_fence_old_frames() -> TestResult {
    let mut test = RecoverySession::ready()?;
    let original = test.lease(22)?;
    let initial = test.frame(original, 1, 0.0, 23);
    assert_eq!(test.enact(initial, 23)?, [Disposition::Accepted]);
    let contract = test.io.adapter.capabilities();
    let released = test.release(30);
    assert!(
        released
            .actions
            .iter()
            .any(|action| matches!(action, SessionAction::EngageLinkLoss { .. }))
    );
    test.enact(released, 30)?;
    assert!(test.io.adapter.status_cached().control_locked);
    test.pose(1, 40)?;
    assert_eq!(test.io.adapter.capabilities(), contract);
    assert!(test.refresh(40).actions.is_empty());

    let recovered = test.lease(41)?;
    assert!(recovered > original);
    let deflected = test.frame(recovered, 2, HOST_PITCH_RATE_LIMIT_RPS, 42);
    assert!(
        !deflected
            .actions
            .iter()
            .any(|action| matches!(action, SessionAction::ClearLinkLoss { .. }))
    );
    assert_eq!(
        test.enact(deflected, 42)?,
        [Disposition::Rejected(RejectReason::LinkLossEngaged)]
    );
    let neutral = test.frame(recovered, 3, 0.0, 43);
    assert!(
        neutral
            .actions
            .iter()
            .any(|action| matches!(action, SessionAction::ClearLinkLoss { .. }))
    );
    assert_eq!(test.enact(neutral, 43)?, [Disposition::Accepted]);
    assert!(!test.io.adapter.status_cached().control_locked);
    for (offset, at) in [(2, 50), (3, 60), (4, 70)] {
        test.pose(offset, at)?;
        assert!(test.refresh(at).actions.is_empty());
        assert_eq!(test.io.adapter.capabilities(), contract);
        assert_eq!(test.lease(at)?, recovered);
    }
    let before = packets(&test.io.written()?).len();
    let moving = test.frame(recovered, 4, HOST_PITCH_RATE_LIMIT_RPS, 71);
    assert_eq!(test.enact(moving, 71)?, [Disposition::Transformed]);
    test.io.step(72);
    assert_eq!(packets(&test.io.written()?).len(), before + 1);
    let written = test.io.written()?;
    let stale = test.frame(original, 5, HOST_PITCH_RATE_LIMIT_RPS, 73);
    assert!(
        stale
            .actions
            .iter()
            .any(|action| matches!(action, SessionAction::RejectFrame { .. }))
    );
    assert!(test.enact(stale, 73)?.is_empty());
    test.io.step(74);
    assert_eq!(test.io.written()?, written);
    Ok(())
}

#[test]
fn feedback_expiry_still_changes_capabilities_revokes_the_lease_and_cancels_pending_motion()
-> TestResult {
    let mut test = RecoverySession::ready()?;
    let generation = test.lease(22)?;
    let ready = test.io.adapter.capabilities();
    let moving = test.frame(generation, 1, HOST_PITCH_RATE_LIMIT_RPS, 23);
    assert_eq!(test.enact(moving, 23)?, [Disposition::Transformed]);
    assert!(test.io.adapter.status_cached().pending_bytes > 0);
    let written = test.io.written()?;
    test.io.step(321);
    assert_eq!(test.io.written()?, written);
    assert_eq!(test.io.adapter.status_cached().pending_bytes, 0);
    let unavailable = test.io.adapter.capabilities();
    let payload = unavailable.vehicles[0].scopes[0].payload.as_ref().unwrap();
    let prior = ready.vehicles[0].scopes[0].payload.as_ref().unwrap();
    assert!(payload.revision > prior.revision);
    assert_eq!(
        payload
            .gimbal
            .as_ref()
            .unwrap()
            .pitch
            .as_ref()
            .unwrap()
            .rate
            .as_ref()
            .unwrap()
            .support,
        wire::PayloadSupportState::Unavailable as i32
    );
    let changed = test.refresh(321);
    assert!(
        changed
            .actions
            .iter()
            .any(|action| matches!(action, SessionAction::InvalidatePendingCommands { .. }))
    );
    assert!(
        changed
            .actions
            .iter()
            .any(|action| matches!(action, SessionAction::EngageLinkLoss { .. }))
    );
    assert!(test.enact(changed, 321).is_err());
    assert!(test.io.adapter.status_cached().control_locked);
    test.renew_window(322)?;
    let stale = test.frame(generation, 2, 0.0, 323);
    assert!(
        stale
            .actions
            .iter()
            .any(|action| matches!(action, SessionAction::RejectFrame { .. }))
    );
    assert!(test.enact(stale, 323)?.is_empty());
    assert_eq!(test.io.written()?, written);
    Ok(())
}
