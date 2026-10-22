use pilotage_adapter_api::{
    AvionicsAttitudeSample, Disposition, MeasurementClock, SourceIncarnation, SourceIntegrity,
    SourceRole, VehicleAdapter,
};
use pilotage_protocol::MotorState;

use super::{
    TestResult,
    harness::{Harness, captured_hex_frames, frame, observed_poses, packets, seal_duml},
};
use crate::adapter::BENCH_PITCH_RATE_LIMIT_RPS;

fn captured_reports() -> Vec<Vec<u8>> {
    captured_hex_frames(include_str!(
        "../airframe/fixtures/mini3-reported-attitude.hex"
    ))
}

#[test]
fn a_late_attachment_keeps_measurements_and_write_deadlines_on_the_host_clock() -> TestResult {
    let mut harness = Harness::mobile();
    let origin = std::time::Instant::now()
        .checked_sub(std::time::Duration::from_secs(300))
        .unwrap();
    harness.adapter = harness.adapter.with_host_clock(origin, None);
    let bytes = captured_reports().remove(0);
    harness.feed(&crate::logiclink::encode_control(&packets(&bytes)[0]))?;
    harness
        .adapter
        .step(pilotage_adapter_api::StepBudget { ticks: 1 });
    let batch = harness.adapter.sample_telemetry();
    let sample = &batch.samples[0];
    let motor = sample.fc_state.unwrap().motor_state.unwrap();
    assert_eq!(motor.state, MotorState::Stopped);
    assert_eq!(motor.stamp.clock, MeasurementClock::HostMonotonic);
    assert!(motor.stamp.acquired_at_ns >= 300_000_000_000);
    assert!(motor.stamp.acquired_at_ns <= origin.elapsed().as_nanos() as u64);
    let stream = harness.adapter.subscribe_encoded_video().unwrap();
    assert_eq!(stream.origin, origin);
    let deadlines = harness.deadlines()?;
    assert!(!deadlines.is_empty());
    assert!(deadlines.iter().all(|deadline| {
        *deadline < std::time::Instant::now() + std::time::Duration::from_secs(1)
    }));
    Ok(())
}

#[test]
fn captured_body_angles_are_reported_euler_without_navigation_or_authorization() -> TestResult {
    let mut harness = Harness::mobile();
    for (index, bytes) in captured_reports().iter().enumerate() {
        let parsed = packets(bytes);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].raw(), bytes);
        harness.feed(&crate::logiclink::encode_control(&parsed[0]))?;
        let millis = 100 * index as u64;
        harness.step(millis);
        let batch = harness.telemetry(millis);
        assert_eq!(batch.samples.len(), 1);
        let sample = &batch.samples[0];
        assert!(sample.payload.is_none() && sample.gimbal.is_none() && sample.battery.is_none());
        assert!(sample.sim_truth.is_none());
        let fc_state = sample.fc_state.unwrap();
        assert_eq!(fc_state.arm_state, 0);
        assert!(fc_state.stamp.is_none());
        assert!(fc_state.last_command.is_none() && fc_state.landed_state.is_none());
        let motor = fc_state.motor_state.unwrap();
        assert_eq!(motor.state, MotorState::Stopped);
        assert_eq!(motor.stamp.role, SourceRole::FcState);
        assert!(sample.pose.is_none() && sample.speed.is_none());
        let avionics = sample.avionics.unwrap();
        assert!(avionics.kinematics.is_none() && avionics.baro.is_none());
        assert!(avionics.geodetic.is_none() && avionics.magnetic_variation.is_none());
        assert!(avionics.estimator_status_stamp.is_none());
        assert_eq!(avionics.valid_flags, 0);
        assert_eq!(avionics.quality, 2);
        let AvionicsAttitudeSample::ReportedEuler {
            pitch_rad,
            roll_rad,
            yaw_rad,
            stamp,
        } = avionics.attitude.unwrap()
        else {
            panic!("reported angles must not become canonical");
        };
        assert!((pitch_rad - (-4.9_f32).to_radians()).abs() < 1e-6);
        assert!((roll_rad - 1.7_f32.to_radians()).abs() < 1e-6);
        assert!((yaw_rad - (-67.6_f32).to_radians()).abs() < 1e-6);
        assert_eq!(stamp.role, SourceRole::OperationalEstimate);
        assert_eq!(stamp.source_id, 0x0e_03_43);
        assert_eq!(stamp.source_incarnation, SourceIncarnation::new([42; 16]));
        assert_eq!(stamp.source_epoch, 0);
        assert_eq!(stamp.integrity, SourceIntegrity::ChecksummedOnly);
        assert_eq!(stamp.clock, MeasurementClock::HostMonotonic);
        assert_eq!(stamp.sequence, index as u32 + 1);
        assert_eq!(stamp.acquired_at_ns, millis * 1_000_000);
        assert_eq!(
            motor.stamp,
            pilotage_adapter_api::MeasurementStamp {
                role: SourceRole::FcState,
                ..stamp
            }
        );
        assert!(harness.telemetry(millis).samples.is_empty());
        assert!(matches!(
            harness.apply(frame(0.0), millis).disposition,
            Disposition::Rejected(_)
        ));
    }
    Ok(())
}

#[test]
fn motor_flag_is_separate_from_flying_arm_and_landed_state() -> TestResult {
    // Positive cases use the SDK bit definition. They are not flight captures.
    for flags in [0_u32, 8, 12, 6, 0xffff_fff7, u32::MAX] {
        let mut bytes = captured_reports().remove(0);
        bytes[43..47].copy_from_slice(&flags.to_le_bytes());
        seal_duml(&mut bytes);
        let mut harness = Harness::mobile();
        harness.feed(&crate::logiclink::encode_control(&packets(&bytes)[0]))?;
        harness.step(0);
        let batch = harness.telemetry(0);
        let sample = &batch.samples[0];
        let fc_state = sample.fc_state.unwrap();
        let expected = if flags & 8 == 0 {
            MotorState::Stopped
        } else {
            MotorState::Running
        };
        assert_eq!(fc_state.motor_state.unwrap().state, expected);
        assert_eq!(fc_state.arm_state, 0);
        assert!(fc_state.stamp.is_none());
        assert!(fc_state.landed_state.is_none() && fc_state.last_command.is_none());
        assert!(sample.flight_procedure.is_none());
        assert!(sample.pose.is_none() && sample.speed.is_none());
        assert!(matches!(
            harness.apply(frame(0.0), 0).disposition,
            Disposition::Rejected(_)
        ));
    }
    Ok(())
}

#[test]
fn wrong_body_routes_versions_lengths_and_angles_do_not_publish() -> TestResult {
    for case in 0..16 {
        let mut bytes = captured_reports().remove(0);
        match case {
            0 => bytes[4] = 3,
            1 => bytes[5] = 10,
            2 => bytes[8] = 0x80,
            3 => bytes[9] = 4,
            4 => bytes[10] = 0x44,
            5 => bytes[2] = 8,
            6 => {
                bytes.remove(80);
            }
            7 => bytes.insert(80, 0),
            8 => bytes.truncate(63),
            9 => bytes.truncate(68),
            _ => {
                let offset = 35 + 2 * ((case - 10) / 2);
                let angle: i16 = if case % 2 == 0 { 1_801 } else { -1_801 };
                bytes[offset..offset + 2].copy_from_slice(&angle.to_le_bytes());
            }
        }
        seal_duml(&mut bytes);
        let mut harness = Harness::new();
        harness.feed(&bytes)?;
        harness.step(0);
        assert!(harness.telemetry(0).samples.is_empty(), "case {case}");
    }
    Ok(())
}

#[test]
fn repeated_or_backwards_body_sequences_do_not_publish_or_renew_age() -> TestResult {
    let reports = captured_reports();
    let mut harness = Harness::new();
    harness.feed(&reports[0])?;
    harness.step(0);
    assert_eq!(
        harness.telemetry(0).samples[0]
            .avionics
            .unwrap()
            .attitude
            .unwrap()
            .stamp()
            .sequence,
        1
    );
    let mut backwards = reports[0].clone();
    let sequence = packets(&reports[0])[0].sequence();
    backwards[6..8].copy_from_slice(&sequence.wrapping_sub(1).to_le_bytes());
    seal_duml(&mut backwards);
    for bytes in [&reports[0], &backwards] {
        harness.feed(bytes)?;
        harness.step(200);
        assert!(harness.telemetry(200).samples.is_empty());
    }
    harness.feed(&reports[1])?;
    harness.step(350);
    let stamp = harness.telemetry(350).samples[0]
        .avionics
        .unwrap()
        .attitude
        .unwrap()
        .stamp();
    assert_eq!(stamp.sequence, 2);
    assert_eq!(stamp.acquired_at_ns, 350_000_000);
    Ok(())
}

#[test]
fn stale_or_future_body_observations_are_withheld() -> TestResult {
    let reports = captured_reports();
    for malformed in [false, true] {
        let mut harness = Harness::new();
        harness.feed(&reports[0])?;
        harness.step(0);
        let mut later = reports[usize::from(malformed)].clone();
        if malformed {
            later[35..37].copy_from_slice(&i16::MAX.to_le_bytes());
            seal_duml(&mut later);
        }
        harness.feed(&later)?;
        harness.step(250);
        assert!(harness.telemetry(301).samples.is_empty());
        let mut fresh = reports[1].clone();
        let sequence = packets(&fresh)[0].sequence().wrapping_add(1);
        fresh[6..8].copy_from_slice(&sequence.to_le_bytes());
        seal_duml(&mut fresh);
        harness.feed(&fresh)?;
        harness.step(310);
        assert!(harness.telemetry(309).samples.is_empty());
        let stamp = harness.telemetry(310).samples[0]
            .avionics
            .unwrap()
            .attitude
            .unwrap()
            .stamp();
        assert_eq!(stamp.sequence, 2);
        assert_eq!(stamp.acquired_at_ns, 310_000_000);
    }
    let mut harness = Harness::new();
    harness.feed(&reports[0])?;
    harness.step(0);
    assert_eq!(harness.telemetry(300).samples.len(), 1);
    Ok(())
}

#[test]
fn body_battery_and_gimbal_have_independent_freshness_and_sequences() -> TestResult {
    let reports = captured_reports();
    let battery = captured_hex_frames(include_str!("../battery/fixtures/mini3-battery-status.hex"));
    let mut harness = Harness::new();
    harness.feed(&reports[0])?;
    harness.feed(&battery[0])?;
    harness.feed(&observed_poses()[0])?;
    harness.step(0);
    let stale_body = harness.telemetry(301);
    assert!(stale_body.samples[0].avionics.is_none());
    assert!(stale_body.samples[0].fc_state.is_none());
    assert!(stale_body.samples[0].payload.is_none());
    assert_eq!(stale_body.samples[0].battery.unwrap().stamp.sequence, 1);
    harness.feed(&reports[1])?;
    harness.step(310);
    let body = harness.telemetry(310);
    assert!(body.samples[0].battery.is_none() && body.samples[0].payload.is_none());
    let stamp = body.samples[0].avionics.unwrap().attitude.unwrap().stamp();
    assert_eq!(stamp.sequence, 2);
    harness.feed(&observed_poses()[1])?;
    harness.step(320);
    let gimbal = harness.telemetry(320);
    assert!(gimbal.samples[0].payload.is_some());
    assert!(gimbal.samples[0].avionics.is_none() && gimbal.samples[0].battery.is_none());
    assert!(gimbal.samples[0].fc_state.is_none());
    harness.feed(&battery[1])?;
    harness.step(330);
    let battery = harness.telemetry(330);
    assert!(battery.samples[0].avionics.is_none() && battery.samples[0].payload.is_none());
    assert!(battery.samples[0].fc_state.is_none());
    assert_eq!(battery.samples[0].battery.unwrap().stamp.sequence, 2);
    Ok(())
}

#[test]
fn bad_crc_or_disconnect_withholds_pending_body_data_and_cannot_resume() -> TestResult {
    let reports = captured_reports();
    for corrupt in [false, true] {
        let mut harness = Harness::new();
        harness.feed(&reports[0])?;
        harness.step(0);
        if corrupt {
            let mut damaged = reports[1].clone();
            damaged[35] ^= 1;
            harness.feed(&damaged)?;
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
fn an_airframe_only_sample_preserves_command_progress_without_payload_angles() -> TestResult {
    let mut harness = Harness::bench();
    harness.feed(&captured_reports()[0])?;
    harness.step(0);
    harness.apply(frame(0.0), 0);
    harness.apply(frame(-BENCH_PITCH_RATE_LIMIT_RPS), 20);
    harness.step(20);
    let batch = harness.adapter.sample_telemetry();
    let sample = &batch.samples[0];
    assert!(sample.avionics.is_some() && sample.battery.is_none());
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

#[test]
fn landing_range_uses_downward_sensor_flags_not_relative_height() -> TestResult {
    for case in 0..8 {
        let mut bytes = captured_reports().remove(0);
        bytes[27..29].copy_from_slice(&500_i16.to_le_bytes());
        bytes[33..35].copy_from_slice(&(-1_i16).to_le_bytes());
        bytes[43..47].copy_from_slice(&0x1c_u32.to_le_bytes());
        bytes[52] = 5;
        match case {
            1 => bytes[43] &= !0x10,
            2 => bytes[45] |= 2,
            3 => bytes[52] = 0,
            4 => bytes[52] = 255,
            5 => bytes[35..37].copy_from_slice(&151_i16.to_le_bytes()),
            6 => bytes[33..35].copy_from_slice(&i16::MIN.to_le_bytes()),
            7 => bytes[59] = 88,
            _ => {}
        }
        seal_duml(&mut bytes);
        let mut h = Harness::mobile();
        h.feed(&crate::logiclink::encode_control(&packets(&bytes)[0]))?;
        h.step(100);
        let approach = h
            .telemetry(100)
            .samples
            .into_iter()
            .find_map(|sample| sample.fc_state.and_then(|fc| fc.landing_approach));
        if case == 0 {
            let approach = approach.unwrap();
            assert_eq!(approach.surface_distance_m, 0.5);
            assert_eq!(approach.vertical_speed_down_mps, -0.1);
            assert!(approach.is_flying);
            assert_eq!(approach.stamp.acquired_at_ns, 100_000_000);
        } else {
            assert!(approach.is_none(), "case {case}");
        }
    }
    Ok(())
}
