use pilotage_adapter_api::{
    Disposition, MeasurementClock, SourceIncarnation, SourceIntegrity, SourceRole, VehicleAdapter,
};

use super::{
    TestResult,
    harness::{Harness, captured_hex_frames, frame, observed_poses, packets, seal_duml as seal},
};
use crate::adapter::HOST_PITCH_RATE_LIMIT_RPS;

fn captured_reports() -> Vec<Vec<u8>> {
    captured_hex_frames(include_str!("../battery/fixtures/mini3-battery-status.hex"))
}

#[test]
fn captured_battery_reports_publish_without_gimbal_over_the_shared_usb_input() -> TestResult {
    let mut harness = Harness::mobile();
    for (index, bytes) in captured_reports().iter().enumerate() {
        let parsed = packets(bytes);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].raw(), bytes);
        harness.feed(&crate::logiclink::encode_control(&parsed[0]))?;
        let millis = 1_000 * index as u64;
        harness.step(millis);
        let batch = harness.telemetry(millis);
        assert_eq!(batch.samples.len(), 1);
        let sample = &batch.samples[0];
        assert!(sample.payload.is_none() && sample.gimbal.is_none());
        assert!(sample.avionics.is_none() && sample.fc_state.is_none());
        assert!(sample.sim_truth.is_none() && sample.pose.is_none() && sample.speed.is_none());
        let battery = sample.battery.unwrap();
        assert_eq!(battery.instance, 0);
        assert_eq!(battery.remaining_fraction, Some(0.08));
        assert_eq!(battery.voltage_v, Some([6.850, 6.848, 6.851][index]));
        assert_eq!(battery.current_a, None);
        assert_eq!(battery.consumed_energy_j, None);
        assert_eq!(battery.time_remaining_s, None);
        assert_eq!(battery.stamp.source_id, 0x09_03_51);
        assert_eq!(battery.stamp.role, SourceRole::FcState);
        assert_eq!(battery.stamp.integrity, SourceIntegrity::ChecksummedOnly);
        assert_eq!(battery.stamp.clock, MeasurementClock::HostMonotonic);
        assert_eq!(
            battery.stamp.source_incarnation,
            SourceIncarnation::new([42; 16])
        );
        assert_eq!(battery.stamp.source_epoch, 0);
        assert_eq!(battery.stamp.sequence, index as u32 + 1);
        assert_eq!(battery.stamp.acquired_at_ns, millis * 1_000_000);
        assert!(harness.telemetry(millis).samples.is_empty());
        assert!(matches!(
            harness.apply(frame(0.0), millis).disposition,
            Disposition::Rejected(_)
        ));
    }
    Ok(())
}

#[test]
fn wrong_battery_routes_layouts_flags_and_out_of_range_values_do_not_publish() -> TestResult {
    for case in 0..13 {
        let mut bytes = captured_reports().remove(0);
        match case {
            0 => bytes[4] = 3,
            1 => bytes[5] = 10,
            2 => bytes[8] = 0x80,
            3 => bytes[9] = 0x0d,
            4 => bytes[10] = 2,
            5 => bytes[2] = 8,
            6 => {
                bytes.remove(40);
            }
            7 => bytes.insert(40, 0),
            8 => bytes[37] = 101,
            9 => bytes[35..37].copy_from_slice(&3_999_u16.to_le_bytes()),
            10 => bytes[35..37].copy_from_slice(&10_001_u16.to_le_bytes()),
            11 => bytes[35..37].copy_from_slice(&0_u16.to_le_bytes()),
            _ => bytes[35..37].copy_from_slice(&u16::MAX.to_le_bytes()),
        }
        seal(&mut bytes);
        let mut harness = Harness::new();
        harness.feed(&bytes)?;
        harness.step(0);
        assert!(harness.telemetry(0).samples.is_empty(), "case {case}");
    }
    Ok(())
}

#[test]
fn battery_deduplication_preserves_its_own_sequence_and_receive_time() -> TestResult {
    let reports = captured_reports();
    let mut harness = Harness::new();
    harness.feed(&reports[0])?;
    harness.step(0);
    assert_eq!(
        harness.telemetry(0).samples[0]
            .battery
            .unwrap()
            .stamp
            .sequence,
        1
    );
    let mut backwards = reports[0].clone();
    let sequence = packets(&reports[0])[0].sequence();
    backwards[6..8].copy_from_slice(&sequence.wrapping_sub(1).to_le_bytes());
    seal(&mut backwards);
    for bytes in [&reports[0], &backwards] {
        harness.feed(bytes)?;
        harness.step(500);
        assert!(harness.telemetry(500).samples.is_empty());
    }
    harness.feed(&reports[1])?;
    harness.step(1_000);
    let stamp = harness.telemetry(1_000).samples[0].battery.unwrap().stamp;
    assert_eq!(stamp.sequence, 2);
    assert_eq!(stamp.acquired_at_ns, 1_000_000_000);
    Ok(())
}

#[test]
fn duplicate_and_malformed_reports_cannot_extend_battery_freshness() -> TestResult {
    let reports = captured_reports();
    for malformed in [false, true] {
        let mut harness = Harness::new();
        harness.feed(&reports[0])?;
        harness.step(0);
        let mut later = reports[usize::from(malformed)].clone();
        if malformed {
            later[37] = 255;
            seal(&mut later);
        }
        harness.feed(&later)?;
        harness.step(2_500);
        assert!(harness.telemetry(3_001).samples.is_empty());
        harness.feed(&reports[1])?;
        harness.step(3_010);
        let stamp = harness.telemetry(3_010).samples[0].battery.unwrap().stamp;
        assert_eq!(stamp.sequence, 2);
        assert_eq!(stamp.acquired_at_ns, 3_010_000_000);
    }
    let mut harness = Harness::new();
    harness.feed(&reports[0])?;
    harness.step(0);
    assert_eq!(harness.telemetry(3_000).samples.len(), 1);
    Ok(())
}

#[test]
fn battery_sequence_wrap_is_forward_and_future_receive_times_are_withheld() -> TestResult {
    let mut harness = Harness::new();
    for (index, sequence) in [u16::MAX, 0].into_iter().enumerate() {
        let mut bytes = captured_reports().remove(0);
        bytes[6..8].copy_from_slice(&sequence.to_le_bytes());
        seal(&mut bytes);
        harness.feed(&bytes)?;
        harness.step(1_000 + index as u64);
        assert!(harness.telemetry(999).samples.is_empty());
        let stamp = harness.telemetry(1_000 + index as u64).samples[0]
            .battery
            .unwrap()
            .stamp;
        assert_eq!(stamp.sequence, index as u32 + 1);
    }
    Ok(())
}

#[test]
fn battery_and_gimbal_updates_do_not_republish_each_other() -> TestResult {
    let mut harness = Harness::new();
    let reports = captured_reports();
    harness.prime()?;
    assert!(harness.telemetry(20).samples[0].payload.is_some());
    harness.feed(&reports[0])?;
    harness.step(1_000);
    let battery_only = harness.telemetry(1_000);
    assert!(battery_only.samples[0].battery.is_some());
    assert!(battery_only.samples[0].payload.is_none());
    let mut pose = observed_poses().remove(2);
    let sequence = packets(&pose)[0].sequence();
    pose[6..8].copy_from_slice(&sequence.wrapping_add(1).to_le_bytes());
    seal(&mut pose);
    harness.feed(&pose)?;
    harness.step(1_010);
    let payload_only = harness.telemetry(1_010);
    assert!(payload_only.samples[0].payload.is_some());
    assert!(payload_only.samples[0].battery.is_none());
    harness.feed(&reports[1])?;
    harness.step(1_020);
    let battery_only = harness.telemetry(1_020);
    assert!(battery_only.samples[0].battery.is_some());
    assert!(battery_only.samples[0].payload.is_none());
    Ok(())
}

#[test]
fn disconnect_or_bad_crc_withholds_an_unpublished_battery_report() -> TestResult {
    let reports = captured_reports();
    for corrupt in [false, true] {
        let mut harness = Harness::new();
        harness.feed(&reports[0])?;
        harness.step(0);
        if corrupt {
            let mut bytes = reports[1].clone();
            bytes[37] ^= 1;
            harness.feed(&bytes)?;
        } else {
            harness.disconnect()?;
        }
        harness.step(10);
        assert!(harness.adapter.status_cached().fault.is_some());
        assert!(harness.telemetry(10).samples.is_empty());
        harness.feed(&reports[2])?;
        harness.step(20);
        assert!(harness.telemetry(20).samples.is_empty());
    }
    Ok(())
}

#[test]
fn a_battery_only_sample_preserves_command_progress_without_inventing_angles() -> TestResult {
    let mut harness = Harness::bench();
    harness.feed(&captured_reports()[0])?;
    harness.step(0);
    harness.apply(frame(0.0), 0);
    harness.apply(frame(-HOST_PITCH_RATE_LIMIT_RPS), 20);
    harness.step(20);
    let batch = harness.adapter.sample_telemetry();
    assert_eq!(batch.samples.len(), 1);
    let sample = &batch.samples[0];
    assert!(sample.battery.is_some());
    let payload = sample.payload.as_ref().unwrap();
    assert!(payload.pitch.is_none() && payload.roll.is_none() && payload.yaw.is_none());
    let progress = payload.command.as_ref().unwrap();
    assert_eq!(
        progress.stage,
        pilotage_protocol::wire::PayloadCommandStage::Sent as i32
    );
    assert!(progress.sent_at.is_some());
    assert!(progress.acknowledged_at.is_none() && progress.observed_complete_at.is_none());
    Ok(())
}
