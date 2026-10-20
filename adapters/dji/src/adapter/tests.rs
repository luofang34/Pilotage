use super::GIMBAL_SCOPE;
mod acknowledgement;
mod airframe;
mod battery;
mod bench;
mod contract;
mod flight;
mod harness;
mod keyframe;
mod mapping;
mod mobile;
mod model;
mod observed;
mod profile;
mod recovery;

use std::time::Duration;

use pilotage_adapter_api::{Disposition, LinkLossPolicy, RejectReason, VehicleAdapter};
use pilotage_protocol::{ControlAction, ControlIntent, GimbalRateIntent, ScopeId};

use super::{DjiFault, HOST_PITCH_RATE_LIMIT_RPS};
use harness::{Harness, frame, observed_poses, packets};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn recorded_cdc_traffic_does_not_authorize_any_control() -> TestResult {
    let mut h = Harness::new();
    h.feed(include_bytes!("../duml/fixtures/c5-cdc-passive.bin"))?;
    h.step(0);
    let capability = h.adapter.capabilities();
    assert_eq!(capability.vehicles[0].scopes.len(), 1);
    assert_eq!(
        capability.vehicles[0].scopes[0].scope.as_str(),
        GIMBAL_SCOPE
    );
    assert!(capability.execution.physically_embodied);
    assert!(!capability.execution.render_capable);
    let rejected = h.apply(frame(-HOST_PITCH_RATE_LIMIT_RPS), 0);
    assert_eq!(
        rejected.disposition,
        Disposition::Rejected(RejectReason::MeasurementUnavailable)
    );
    assert!(h.telemetry(0).samples.is_empty());
    assert_eq!(h.adapter.status_cached().received_bytes, 228);
    let written = h.written()?;
    let probe = packets(&written);
    assert_eq!(probe.len(), 1);
    assert_eq!((probe[0].sender().0, probe[0].receiver().0), (10, 6));
    assert_eq!((probe[0].command_set(), probe[0].command_id()), (0, 1));
    assert!(probe[0].payload().is_empty());
    assert_eq!(
        h.recorded("rx")?,
        include_bytes!("../duml/fixtures/c5-cdc-passive.bin")
    );
    assert_eq!(h.recorded("tx")?, written);
    Ok(())
}

#[test]
fn recorded_poses_publish_payload_only_and_bound_pitch_commands() -> TestResult {
    let mut h = Harness::new();
    h.prime()?;
    let samples = h.telemetry(20).samples;
    assert_eq!(samples.len(), 1);
    let sample = &samples[0];
    assert!(sample.pose.is_none() && sample.speed.is_none() && sample.avionics.is_none());
    assert!(sample.fc_state.is_none() && sample.sim_truth.is_none() && sample.battery.is_none());
    assert!(sample.gimbal.is_none());
    let payload = sample.payload.as_ref().ok_or("missing payload sample")?;
    let pitch = payload.pitch.as_ref().ok_or("missing pitch")?;
    let stamp = pitch.stamp.as_ref().ok_or("missing stamp")?;
    assert_eq!(
        stamp.role,
        pilotage_protocol::wire::SourceRole::PayloadDevice as i32
    );
    assert_eq!(
        stamp.clock,
        pilotage_protocol::wire::MeasurementClock::HostMonotonic as i32
    );
    assert_eq!(
        pitch.received_at.as_ref().map(|time| time.nanos),
        Some(20_000_000)
    );
    assert!(pitch.sampled_at.is_none());
    assert!(pitch.rate_rad_s.is_none());
    assert_eq!(
        pitch.frame,
        pilotage_protocol::wire::PayloadReferenceFrame::Unspecified as i32
    );
    assert!((pitch.angle_rad.ok_or("missing angle")?.to_degrees() + 4.9).abs() < 0.001);
    assert!(h.telemetry(20).samples.is_empty());
    assert_eq!(
        h.apply(frame(-HOST_PITCH_RATE_LIMIT_RPS), 20).disposition,
        Disposition::Transformed
    );
    h.step(30);
    let written = h.written()?;
    let commands = packets(&written);
    assert_eq!(commands.len(), 2);
    let move_packet = &commands[1];
    assert_eq!((move_packet.sender().0, move_packet.receiver().0), (2, 4));
    assert_eq!(
        (move_packet.command_set(), move_packet.command_id()),
        (4, 0x14)
    );
    assert_eq!(move_packet.payload(), &[0, 0, 0, 0, 163, 254, 7, 10]);
    assert_eq!(h.recorded("tx")?, written);
    Ok(())
}

#[test]
fn repeated_sequences_do_not_renew_freshness_or_admit_control() -> TestResult {
    let mut h = Harness::new();
    let pose = observed_poses().remove(0);
    for ms in [0, 10, 20] {
        h.feed(&pose)?;
        h.step(ms);
    }
    assert_eq!(h.adapter.status_cached().consecutive_poses, 1);
    assert!(matches!(
        h.apply(frame(0.0), 20).disposition,
        Disposition::Rejected(_)
    ));
    h = Harness::new();
    h.prime()?;
    let last = observed_poses().remove(2);
    h.feed(&last)?;
    h.step(321);
    assert!(matches!(
        h.apply(frame(0.0), 321).disposition,
        Disposition::Rejected(RejectReason::MeasurementUnavailable)
    ));
    assert!(h.telemetry(321).samples.is_empty());
    Ok(())
}

#[test]
fn unsupported_commands_never_reach_the_serial_writer() -> TestResult {
    let mut h = Harness::new();
    h.prime()?;
    let before = h.written()?;
    let mut yaw = frame(0.0);
    yaw.intent = Some(ControlIntent::GimbalRate(GimbalRateIntent {
        pitch_rate: 0.0,
        yaw_rate: HOST_PITCH_RATE_LIMIT_RPS * 2.0,
    }));
    let mut arm = frame(0.0);
    arm.actions.push(ControlAction::Arm);
    let mut flight = frame(0.0);
    flight.scope = ScopeId::new("vehicle.motion");
    let mut land = flight.clone();
    land.intent = None;
    land.actions = vec![ControlAction::Land];
    for request in [
        yaw,
        arm,
        flight,
        land,
        frame(f32::NAN),
        frame(HOST_PITCH_RATE_LIMIT_RPS * 2.0),
    ] {
        assert!(matches!(
            h.apply(request, 20).disposition,
            Disposition::Rejected(_)
        ));
        h.step(20);
    }
    assert_eq!(h.written()?, before);
    Ok(())
}

#[test]
fn recenter_is_pitch_zero_and_neutral_ticks_do_not_cancel_it() -> TestResult {
    let mut h = Harness::new();
    h.prime()?;
    let mut recenter = frame(0.0);
    recenter.actions.push(ControlAction::GimbalRecenter);
    let result = h.apply(recenter, 20);
    assert_eq!(result.disposition, Disposition::Accepted);
    assert!(result.action_results[0].accepted);
    h.step(30);
    let written = h.written()?;
    assert_eq!(packets(&written)[1].payload(), &[0, 0, 0, 0, 0, 0, 7, 10]);
    for ms in [40, 50, 60] {
        assert_eq!(h.apply(frame(0.0), ms).disposition, Disposition::Accepted);
        h.step(ms);
    }
    assert_eq!(h.written()?, written);
    Ok(())
}

#[test]
fn short_writes_keep_one_packet_and_record_only_actual_bytes() -> TestResult {
    let mut h = Harness::new();
    h.prime()?;
    h.set_write_limit(3)?;
    assert_eq!(
        h.apply(frame(-HOST_PITCH_RATE_LIMIT_RPS), 20).disposition,
        Disposition::Transformed
    );
    h.step(30);
    assert_eq!(h.adapter.status_cached().pending_bytes, 18);
    assert_eq!(
        h.apply(frame(-HOST_PITCH_RATE_LIMIT_RPS), 30).disposition,
        Disposition::Transformed
    );
    for ms in [40, 50, 60, 70, 80, 90] {
        h.step(ms);
    }
    let written = h.written()?;
    assert_eq!(packets(&written).len(), 2);
    assert_eq!(h.adapter.status_cached().pending_bytes, 0);
    assert_eq!(h.recorded("tx")?, written);
    Ok(())
}

#[test]
fn link_loss_closes_a_partial_packet_and_never_reconnects() -> TestResult {
    let mut h = Harness::new();
    h.prime()?;
    h.set_write_limit(3)?;
    h.apply(frame(-HOST_PITCH_RATE_LIMIT_RPS), 20);
    h.step(30);
    let written = h.written()?;
    assert!(h.link_loss(Some(LinkLossPolicy::Neutralize), 30).is_err());
    assert!(matches!(
        h.adapter.status_cached().fault,
        Some(DjiFault::PartialCommand)
    ));
    assert!(h.link_loss(None, 30).is_err());
    assert_eq!(
        h.apply(frame(0.0), 30).disposition,
        Disposition::Rejected(RejectReason::LinkLossEngaged)
    );
    h.feed(&observed_poses().concat())?;
    h.set_write_limit(usize::MAX)?;
    h.step(40);
    assert_eq!(h.written()?, written);
    Ok(())
}

#[test]
fn stale_link_loss_fails_closed_and_fresh_link_loss_writes_a_finite_hold() -> TestResult {
    let mut h = Harness::new();
    h.prime()?;
    assert!(h.link_loss(Some(LinkLossPolicy::Neutralize), 321).is_err());
    assert!(h.link_loss(None, 321).is_err());
    assert_eq!(
        h.apply(frame(0.0), 321).disposition,
        Disposition::Rejected(RejectReason::LinkLossEngaged)
    );
    h = Harness::new();
    h.prime()?;
    h.link_loss(Some(LinkLossPolicy::Neutralize), 20)?;
    assert_eq!(
        packets(&h.written()?)[1].payload(),
        &[0, 0, 0, 0, 207, 255, 7, 10]
    );
    h.link_loss(None, 20)?;
    assert_eq!(h.apply(frame(0.0), 20).disposition, Disposition::Accepted);
    Ok(())
}

#[test]
fn bad_crc_disconnect_and_recording_errors_close_control() -> TestResult {
    let mut h = Harness::new();
    h.prime()?;
    let mut corrupt = observed_poses().remove(2);
    corrupt[20] ^= 1;
    h.feed(&corrupt)?;
    h.step(30);
    assert!(matches!(
        h.adapter.status_cached().fault,
        Some(DjiFault::Framing)
    ));
    assert!(matches!(
        h.apply(frame(0.0), 30).disposition,
        Disposition::Rejected(_)
    ));
    assert!(h.telemetry(30).samples.is_empty());
    let mut h = Harness::new();
    h.prime()?;
    h.disconnect()?;
    h.step(30);
    assert!(matches!(
        h.adapter.status_cached().fault,
        Some(DjiFault::Transport { .. })
    ));
    let mut h = Harness::new();
    h.prime()?;
    h.fail_capture()?;
    h.apply(frame(-HOST_PITCH_RATE_LIMIT_RPS), 20);
    h.step(30);
    assert!(matches!(
        h.adapter.status_cached().fault,
        Some(DjiFault::Recording { .. })
    ));
    let written = h.written()?;
    h.apply(frame(HOST_PITCH_RATE_LIMIT_RPS), 30);
    h.step(40);
    assert_eq!(h.written()?, written);
    Ok(())
}

#[test]
fn unknown_modes_do_not_publish_or_qualify_a_later_report() -> TestResult {
    let mut h = Harness::new();
    for (index, mut pose) in observed_poses().into_iter().enumerate() {
        if index < 2 {
            pose[17] = 0;
        }
        let length = pose.len();
        let crc = turret_dji::frame_checksum(&pose[..length - 2]).to_le_bytes();
        pose[length - 2..].copy_from_slice(&crc);
        h.feed(&pose)?;
        h.step(index as u64 * 10);
        if index < 2 {
            assert!(h.telemetry(index as u64 * 10).samples.is_empty());
        }
    }
    // The last report has a known reference, but the earlier modes cannot
    // qualify it for commands. Publishing one real attitude is independent.
    assert_eq!(h.telemetry(20).samples.len(), 1);
    assert_eq!(h.adapter.status_cached().consecutive_poses, 1);
    assert!(matches!(
        h.apply(frame(0.0), 20).disposition,
        Disposition::Rejected(_)
    ));
    Ok(())
}

#[test]
fn read_work_is_bounded_and_a_partial_frame_blocks_control() -> TestResult {
    let mut h = Harness::new();
    let traffic = include_bytes!("../duml/fixtures/c5-cdc-passive.bin").repeat(300);
    h.set_read_limit(131)?;
    h.feed(&traffic)?;
    h.step(0);
    assert_eq!(h.adapter.status_cached().received_bytes, 65_536);
    h.step(10);
    assert_eq!(
        h.adapter.status_cached().received_bytes,
        traffic.len() as u64
    );
    h = Harness::new();
    h.prime()?;
    h.feed(&observed_poses()[2][..7])?;
    h.step(30);
    assert!(matches!(
        h.apply(frame(0.0), 30).disposition,
        Disposition::Rejected(_)
    ));
    assert!(h.adapter.status_cached().fault.is_none());
    assert!(h.adapter.observations.ready(Duration::from_millis(30)));
    Ok(())
}

#[test]
fn coalesced_recorded_poses_need_new_arrivals_before_control() -> TestResult {
    let mut h = Harness::new();
    h.feed(&observed_poses().concat())?;
    h.step(0);
    assert_eq!(h.adapter.status_cached().consecutive_poses, 1);
    assert!(matches!(
        h.apply(frame(0.0), 0).disposition,
        Disposition::Rejected(_)
    ));
    for millis in [10, 20] {
        h.step(millis);
    }
    assert_eq!(h.adapter.status_cached().consecutive_poses, 1);
    assert!(matches!(
        h.apply(frame(0.0), 20).disposition,
        Disposition::Rejected(_)
    ));
    h = Harness::new();
    h.prime()?;
    assert_eq!(h.adapter.status_cached().consecutive_poses, 3);
    assert_eq!(h.apply(frame(0.0), 20).disposition, Disposition::Accepted);
    Ok(())
}

#[test]
fn every_qualifying_arrival_must_be_inside_the_pitch_envelope() -> TestResult {
    let mut h = Harness::new();
    for (index, mut pose) in observed_poses().into_iter().enumerate() {
        if index == 1 {
            pose[11..13].copy_from_slice(&(-901_i16).to_le_bytes());
            let length = pose.len();
            let crc = turret_dji::frame_checksum(&pose[..length - 2]).to_le_bytes();
            pose[length - 2..].copy_from_slice(&crc);
        }
        h.feed(&pose)?;
        h.step(index as u64 * 10);
    }
    assert_eq!(h.adapter.status_cached().consecutive_poses, 1);
    assert!(matches!(
        h.apply(frame(0.0), 20).disposition,
        Disposition::Rejected(_)
    ));
    Ok(())
}

#[test]
fn an_initial_backlog_cannot_qualify_by_spanning_read_budgets() -> TestResult {
    let mut h = Harness::new();
    let mut backlog = Vec::new();
    let template = observed_poses().remove(0);
    for sequence in 0_u16..3000 {
        let mut pose = template.clone();
        pose[6..8].copy_from_slice(&sequence.to_le_bytes());
        let length = pose.len();
        let crc = turret_dji::frame_checksum(&pose[..length - 2]).to_le_bytes();
        pose[length - 2..].copy_from_slice(&crc);
        backlog.extend(pose);
    }
    h.feed(&backlog)?;
    for millis in [0, 10, 20] {
        h.step(millis);
    }
    assert_eq!(
        h.adapter.status_cached().received_bytes,
        backlog.len() as u64
    );
    assert_eq!(h.adapter.status_cached().consecutive_poses, 1);
    assert!(matches!(
        h.apply(frame(0.0), 20).disposition,
        Disposition::Rejected(_)
    ));
    Ok(())
}

#[test]
fn time_spent_reading_and_recording_cannot_extend_command_freshness() -> TestResult {
    let mut h = Harness::new();
    h.prime()?;
    let before = h.written()?;
    assert_eq!(
        h.apply(frame(-HOST_PITCH_RATE_LIMIT_RPS), 20).disposition,
        Disposition::Transformed
    );
    let mut now = Duration::from_millis(30);
    h.adapter.step_with_time_blocking(|| {
        let result = now;
        now = Duration::from_millis(400);
        result
    });
    assert_eq!(h.written()?, before);
    assert_eq!(h.adapter.status_cached().pending_bytes, 0);
    let mut now = Duration::from_millis(30);
    assert!(
        h.adapter
            .link_loss_with_time_blocking(
                pilotage_protocol::VehicleId::new(1),
                &ScopeId::new(GIMBAL_SCOPE),
                Some(LinkLossPolicy::Neutralize),
                || {
                    let result = now;
                    now = Duration::from_millis(400);
                    result
                },
            )
            .is_err()
    );
    assert_eq!(h.written()?, before);
    Ok(())
}

#[test]
fn an_injected_missing_mode_cannot_reuse_the_last_valid_attitude() -> TestResult {
    let mut h = Harness::new();
    h.prime()?;
    let mut short = observed_poses()[2][..17].to_vec();
    short[1] = 19;
    short[3] = turret_dji::header_checksum(&short[..3]);
    short[6..8].copy_from_slice(&16072_u16.to_le_bytes());
    short.extend(turret_dji::frame_checksum(&short).to_le_bytes());
    h.feed(&short)?;
    h.step(30);
    assert_eq!(h.adapter.status_cached().consecutive_poses, 0);
    assert!(h.telemetry(30).samples.is_empty());
    assert!(matches!(
        h.apply(frame(0.0), 30).disposition,
        Disposition::Rejected(_)
    ));
    Ok(())
}
