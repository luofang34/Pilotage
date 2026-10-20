use pilotage_adapter_api::{LinkLossPolicy, VehicleAdapter};
use pilotage_protocol::wire;

use super::{
    TestResult,
    harness::{Harness, frame, packets},
};
use crate::adapter::HOST_PITCH_RATE_LIMIT_RPS;

type Ready = Result<(Harness, Vec<u8>), Box<dyn std::error::Error>>;

fn ready() -> Ready {
    let mut harness = Harness::bench();
    harness.step(0);
    harness.apply(frame(0.0), 0);
    harness.apply(frame(-HOST_PITCH_RATE_LIMIT_RPS), 20);
    harness.step(20);
    let sent = packets(&harness.written()?);
    let command = sent.last().ok_or("missing pitch command")?;
    let mut response = include_bytes!("../../gimbal/fixtures/pitch-response.bin").to_vec();
    response[6..8].copy_from_slice(&command.sequence().to_le_bytes());
    checksum(&mut response);
    Ok((harness, response))
}

fn checksum(bytes: &mut [u8]) {
    let length = bytes.len();
    let checksum = turret_dji::frame_checksum(&bytes[..length - 2]).to_le_bytes();
    bytes[length - 2..].copy_from_slice(&checksum);
}

#[test]
fn a_correlated_zero_response_is_acknowledgement_without_pose_or_completion() -> TestResult {
    let (mut harness, response) = ready()?;
    harness.feed(&response)?;
    harness.step(50);
    let batch = harness.adapter.sample_telemetry();
    let state = batch.samples[0].payload.as_ref().unwrap();
    assert!(state.pitch.is_none() && state.roll.is_none() && state.yaw.is_none());
    let progress = state.command.as_ref().unwrap();
    assert_eq!(
        progress.stage,
        wire::PayloadCommandStage::DeviceAccepted as i32
    );
    assert_eq!(progress.sent_at.as_ref().unwrap().nanos, 20_000_000);
    assert_eq!(progress.acknowledged_at.as_ref().unwrap().nanos, 50_000_000);
    assert!(progress.observed_complete_at.is_none());
    assert!(progress.pitch_target_rad.unwrap() < 0.0);
    assert!(progress.detail.contains("measured completion"));
    Ok(())
}

#[test]
fn stale_wrong_route_sequence_and_unverified_codes_do_not_acknowledge() -> TestResult {
    for case in 0..5 {
        let (mut harness, mut response) = ready()?;
        let mut at = 50;
        match case {
            0 => response[5] = 10,
            1 => response[6] = response[6].wrapping_add(1),
            2 => response[11] = 0xe0,
            3 => at = 1021,
            _ => {
                assert!(
                    harness
                        .link_loss(Some(LinkLossPolicy::Neutralize), 30)
                        .is_err()
                );
            }
        }
        checksum(&mut response);
        harness.feed(&response)?;
        harness.step(at);
        let progress = harness
            .adapter
            .connection
            .execution
            .wire_progress()
            .unwrap();
        assert!(progress.acknowledged_at.is_none(), "case {case}");
        assert!(progress.observed_complete_at.is_none(), "case {case}");
    }
    Ok(())
}

#[test]
fn a_response_before_full_write_cannot_acknowledge_pending_bytes() -> TestResult {
    let (mut harness, mut response) = ready()?;
    harness.set_write_limit(0)?;
    harness.apply(frame(-HOST_PITCH_RATE_LIMIT_RPS), 40);
    response[6] = response[6].wrapping_add(1);
    checksum(&mut response);
    harness.feed(&response)?;
    harness.step(50);
    let progress = harness
        .adapter
        .connection
        .execution
        .wire_progress()
        .unwrap();
    assert!(progress.sent_at.is_none() && progress.acknowledged_at.is_none());
    harness.set_write_limit(usize::MAX)?;
    harness.step(60);
    let progress = harness
        .adapter
        .connection
        .execution
        .wire_progress()
        .unwrap();
    assert!(progress.sent_at.is_some() && progress.acknowledged_at.is_none());
    Ok(())
}
