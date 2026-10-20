use std::time::Duration;

use pilotage_adapter_api::{Disposition, VehicleAdapter};
use pilotage_protocol::{ControlIntent, Generation, GimbalRateIntent, wire};

use super::{
    TestResult,
    harness::{Harness, frame},
};
use crate::adapter::HOST_PITCH_RATE_LIMIT_RPS;

fn initialized() -> TestResultHarness {
    let mut harness = Harness::bench();
    harness.step(0);
    harness.apply(frame(0.0), 0);
    Ok(harness)
}

type TestResultHarness = Result<Harness, Box<dyn std::error::Error>>;

#[test]
fn a_host_budget_is_consumed_at_the_final_writer() -> TestResult {
    let mut harness = initialized()?;
    let mut request = frame(-HOST_PITCH_RATE_LIMIT_RPS);
    request.admitted_timing.as_mut().unwrap().send_remaining = Duration::from_millis(5);
    harness.apply(request, 20);
    let before = harness.written()?;
    harness.step(25);
    assert_eq!(harness.written()?, before);
    let progress = harness
        .adapter
        .connection
        .execution
        .wire_progress()
        .unwrap();
    assert_eq!(progress.stage, wire::PayloadCommandStage::Expired as i32);
    assert!(progress.sent_at.is_none());
    Ok(())
}

#[test]
fn unadmitted_input_and_unimplemented_yaw_never_write() -> TestResult {
    let mut harness = initialized()?;
    let before = harness.written()?;
    let mut missing = frame(-HOST_PITCH_RATE_LIMIT_RPS);
    missing.admitted_timing = None;
    let mut yaw = frame(0.0);
    yaw.intent = Some(ControlIntent::GimbalRate(GimbalRateIntent {
        pitch_rate: 0.0,
        yaw_rate: HOST_PITCH_RATE_LIMIT_RPS / 2.0,
    }));
    for request in [missing, yaw] {
        assert!(matches!(
            harness.apply(request, 20).disposition,
            Disposition::Rejected(_)
        ));
        harness.step(20);
    }
    assert_eq!(harness.written()?, before);
    Ok(())
}

#[test]
fn a_capture_failure_preserves_the_fact_that_bytes_were_sent() -> TestResult {
    let mut harness = initialized()?;
    harness.apply(frame(-HOST_PITCH_RATE_LIMIT_RPS), 20);
    harness.fail_capture()?;
    harness.step(20);
    let progress = harness
        .adapter
        .connection
        .execution
        .wire_progress()
        .unwrap();
    assert!(progress.sent_at.is_some());
    assert!(progress.acknowledged_at.is_none());
    assert!(progress.observed_complete_at.is_none());
    assert!(progress.pitch_target_rad.is_some());
    assert!(harness.adapter.status_cached().control_locked);
    Ok(())
}

#[test]
fn command_telemetry_never_invents_a_measured_axis() -> TestResult {
    let mut harness = initialized()?;
    harness.apply(frame(-HOST_PITCH_RATE_LIMIT_RPS), 20);
    harness.step(20);
    let telemetry = harness.adapter.sample_telemetry();
    let sample = &telemetry.samples[0];
    let state = sample.payload.as_ref().unwrap();
    assert!(state.pitch.is_none() && state.roll.is_none() && state.yaw.is_none());
    assert!(sample.gimbal.is_none() && sample.pose.is_none());
    let progress = state.command.as_ref().unwrap();
    assert_eq!(progress.stage, wire::PayloadCommandStage::Sent as i32);
    assert!(progress.pitch_target_rad.unwrap() < 0.0);
    assert!(progress.acknowledged_at.is_none() && progress.observed_complete_at.is_none());
    Ok(())
}

#[test]
fn a_new_authority_fences_an_unsent_old_target() -> TestResult {
    let mut harness = initialized()?;
    harness.apply(frame(-HOST_PITCH_RATE_LIMIT_RPS), 20);
    let before = harness.written()?;
    let mut new_owner = frame(0.0);
    new_owner.generation = Generation::new(2);
    harness.apply(new_owner, 25);
    harness.step(30);
    assert_eq!(harness.written()?, before);
    assert!(matches!(
        harness
            .apply(frame(-HOST_PITCH_RATE_LIMIT_RPS), 40)
            .disposition,
        Disposition::Rejected(_)
    ));
    harness.step(40);
    assert_eq!(harness.written()?, before);
    Ok(())
}

#[test]
fn the_catalog_and_state_share_the_attachment_identity() -> TestResult {
    let mut harness = initialized()?;
    harness.apply(frame(-HOST_PITCH_RATE_LIMIT_RPS), 20);
    harness.step(20);
    let catalog = harness.adapter.capabilities();
    let capability = catalog.vehicles[0].scopes[0].payload.as_ref().unwrap();
    let batch = harness.adapter.sample_telemetry();
    let state = batch.samples[0].payload.as_ref().unwrap();
    assert_eq!(capability.connection_incarnation, vec![42; 16]);
    assert_eq!(
        capability.connection_incarnation,
        state.connection_incarnation
    );
    assert_eq!(capability.connection_epoch, state.connection_epoch);
    assert_eq!(capability.revision, state.capability_revision);
    assert_eq!(
        capability.connection_role,
        wire::PayloadConnectionRole::DirectDevice as i32
    );
    Ok(())
}

#[test]
fn feedback_availability_does_not_invent_yaw_control() -> TestResult {
    let mut harness = Harness::new();
    let before = *harness.adapter.connection.execution.capabilities();
    assert_eq!(
        before.gimbal.axes.yaw.feedback.angle,
        turret_core::Capability::Unknown
    );
    harness.prime()?;
    let observed = *harness.adapter.connection.execution.capabilities();
    assert!(observed.revision > before.revision);
    assert_eq!(
        observed.gimbal.axes.yaw.feedback.angle,
        turret_core::Capability::Available(())
    );
    assert_eq!(
        observed.gimbal.axes.yaw.rate,
        turret_core::Capability::Unsupported
    );
    harness.step(321);
    let stale = harness.adapter.connection.execution.capabilities();
    assert!(stale.revision > observed.revision);
    assert!(matches!(
        stale.gimbal.axes.pitch.position,
        turret_core::Capability::Unavailable { .. }
    ));
    assert_eq!(
        stale.gimbal.axes.yaw.feedback.angle,
        turret_core::Capability::Unavailable {
            capability: (),
            reason: turret_core::UnavailableReason::MissingFeedback,
        }
    );
    Ok(())
}

#[test]
fn missing_write_receipt_is_unknown_not_a_claim_of_unsent_cancellation() -> TestResult {
    let mut harness = initialized()?;
    harness.apply(frame(-HOST_PITCH_RATE_LIMIT_RPS), 20);
    let before = harness.written()?.len();
    harness.lose_write_receipt()?;
    harness.step(20);
    assert!(harness.written()?.len() > before);
    let progress = harness
        .adapter
        .connection
        .execution
        .wire_progress()
        .unwrap();
    assert_eq!(
        progress.stage,
        wire::PayloadCommandStage::OutcomeUnknown as i32
    );
    assert_eq!(
        progress.end_reason,
        wire::PayloadEndReason::TransportFailure as i32
    );
    assert!(progress.sent_at.is_none());
    assert!(harness.adapter.status_cached().control_locked);
    let written = harness.written()?;
    harness.step(30);
    assert_eq!(harness.written()?, written);
    Ok(())
}
