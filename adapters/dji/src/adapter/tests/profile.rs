use std::time::Duration;

use pilotage_adapter_api::VehicleAdapter;
use turret_core::{AngularRate, Capability};
use turret_dji::{Mini3ControlProfile, PitchPlanner};

use super::{
    TestResult,
    harness::{Harness, observed_poses, packets},
};
use crate::adapter::{DjiFault, HOST_PITCH_RATE_LIMIT_RPS};

#[test]
fn both_modes_advertise_the_selected_host_policy_without_a_hardware_speed_claim() -> TestResult {
    for (harness, minimum) in [(Harness::new(), -50), (Harness::bench(), -900)] {
        let profile = harness.adapter.profile;
        assert_eq!(profile.minimum_tenths(), minimum);
        assert_eq!(
            profile.rate_limit().as_radians_per_second(),
            HOST_PITCH_RATE_LIMIT_RPS
        );
        let pitch = harness
            .adapter
            .connection
            .execution
            .capabilities()
            .gimbal
            .axes
            .pitch;
        let position = match pitch.position {
            Capability::Available(value)
            | Capability::Unavailable {
                capability: value, ..
            } => value,
            _ => panic!("pitch position contract is absent"),
        };
        let rate = match pitch.rate {
            Capability::Available(value)
            | Capability::Unavailable {
                capability: value, ..
            } => value,
            _ => panic!("pitch rate contract is absent"),
        };
        assert!(position.maximum_rate.is_none());
        assert_eq!(rate.maximum_rate, profile.rate_limit());
        assert!(profile.pitch_target(minimum).is_ok());
        assert!(profile.pitch_target(minimum - 1).is_err());
        let advertised = harness.adapter.capabilities();
        assert_eq!(
            advertised.vehicles[0].scopes[0].intents[0].max_angular,
            profile.rate_limit().as_radians_per_second()
        );
    }
    Ok(())
}

#[test]
fn observed_continuity_uses_the_selected_profile_range() -> TestResult {
    for (mut harness, expected) in [(Harness::new(), 0), (Harness::bench(), 3)] {
        for (index, mut pose) in observed_poses().into_iter().enumerate() {
            pose[11..13].copy_from_slice(&(-51_i16).to_le_bytes());
            let end = pose.len() - 2;
            let checksum = turret_dji::frame_checksum(&pose[..end]).to_le_bytes();
            pose[end..].copy_from_slice(&checksum);
            harness.feed(&pose)?;
            harness.step(index as u64 * 10);
        }
        assert_eq!(harness.adapter.status_cached().consecutive_poses, expected);
    }
    Ok(())
}

#[test]
fn a_target_from_a_wider_profile_latches_an_encoding_fault_before_admission() -> TestResult {
    let mut harness = Harness::mobile();
    let mut video = harness
        .adapter
        .subscribe_encoded_video()
        .ok_or("missing video receiver")?;
    for (index, pose) in observed_poses().iter().enumerate() {
        harness.feed(&crate::logiclink::encode_control(&packets(pose)[0]))?;
        harness.step(index as u64 * 10);
    }
    let before = harness.written()?;
    let wider = Mini3ControlProfile::new(
        -900,
        AngularRate::radians_per_second(HOST_PITCH_RATE_LIMIT_RPS)?,
    )?;
    assert!(!harness.adapter.connection.queue_pitch_until(
        wider.pitch_target(-51)?,
        7,
        Duration::from_millis(20),
        Duration::from_millis(100),
    ));
    harness.step(30);
    assert_eq!(harness.written()?, before);
    assert!(matches!(
        harness.adapter.status_cached().fault,
        Some(DjiFault::Profile {
            operation: "encode pitch",
            source,
        }) if source.tenths == -51 && source.minimum_tenths == -50
    ));
    assert!(harness.adapter.status_cached().control_locked);
    assert!(matches!(
        video.receiver.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
    ));
    assert!(
        harness
            .adapter
            .connection
            .execution
            .wire_progress()
            .is_none()
    );
    Ok(())
}

#[test]
fn a_written_target_rejected_by_the_planner_preserves_send_evidence_and_locks_control() -> TestResult
{
    let mut harness = Harness::bench();
    harness.step(0);
    let before = harness.written()?;
    assert!(harness.adapter.connection.queue_pitch_until(
        harness.adapter.profile.pitch_target(-60)?,
        7,
        Duration::from_millis(10),
        Duration::from_millis(100),
    ));
    let narrower = Mini3ControlProfile::new(
        -50,
        AngularRate::radians_per_second(HOST_PITCH_RATE_LIMIT_RPS)?,
    )?;
    harness.adapter.control.bench = Some(PitchPlanner::from_confirmed_horizontal(narrower));
    harness.step(20);
    assert!(harness.written()?.len() > before.len());
    assert!(matches!(
        harness.adapter.status_cached().fault,
        Some(DjiFault::Profile {
            operation: "record written pitch",
            source,
        }) if source.tenths == -60 && source.minimum_tenths == -50
    ));
    assert!(harness.adapter.status_cached().control_locked);
    assert_eq!(
        harness.adapter.status_cached().commanded_pitch_tenths,
        Some(0)
    );
    let progress = harness
        .adapter
        .connection
        .execution
        .wire_progress()
        .ok_or("missing send evidence")?;
    assert!(progress.sent_at.is_some());
    assert!(progress.acknowledged_at.is_none());
    assert!(progress.observed_complete_at.is_none());
    Ok(())
}
