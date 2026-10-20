mod expiry;

use std::time::Duration;

use pilotage_adapter_api::{Disposition, LinkLossPolicy, VehicleAdapter};
use pilotage_protocol::ControlAction;

use super::super::{BENCH_PITCH_RATE_LIMIT_RPS, DjiControlMode};
use super::{
    TestResult,
    harness::{Harness, frame, packets},
    mapping::mapped_frame,
};

type HarnessResult = Result<Harness, Box<dyn std::error::Error>>;

fn initialized() -> HarnessResult {
    let mut h = Harness::bench();
    h.link_loss(None, 0)?;
    h.feed(include_bytes!("../../duml/fixtures/c5-cdc-passive.bin"))?;
    h.step(0);
    assert_eq!(h.apply(frame(0.0), 0).disposition, Disposition::Accepted);
    Ok(h)
}

#[cfg(test)]
fn targets(h: &Harness) -> Result<Vec<i16>, Box<dyn std::error::Error>> {
    let mut result = Vec::new();
    for packet in packets(&h.written()?) {
        if packet.command_set() == 0 {
            assert_eq!(
                (packet.command_id(), packet.sender().0, packet.receiver().0),
                (1, 10, 6)
            );
            continue;
        }
        assert_eq!((packet.command_set(), packet.command_id()), (4, 0x14));
        assert_eq!((packet.sender().0, packet.receiver().0), (2, 4));
        assert_eq!(&packet.payload()[..4], &[0; 4]);
        assert_eq!(&packet.payload()[6..], &[7, 10]);
        let pitch = i16::from_le_bytes([packet.payload()[4], packet.payload()[5]]);
        assert!((-900..=0).contains(&pitch));
        result.push(pitch);
    }
    Ok(result)
}

#[test]
fn explicit_mode_has_no_startup_motion_fake_pose_or_neutralize_claim() -> TestResult {
    let mut h = Harness::bench();
    h.link_loss(None, 0)?;
    assert!(h.written()?.is_empty());
    h.step(0);
    let caps = h.adapter.capabilities();
    assert!(caps.vehicles[0].display_name.contains("one-way bench"));
    assert!(caps.vehicles[0].link_loss_actions.is_empty());
    assert_eq!(
        h.adapter.status_cached().control_mode,
        DjiControlMode::BenchInitialHorizontal
    );
    assert_eq!(h.adapter.status_cached().commanded_pitch_tenths, Some(0));
    assert!(h.adapter.status_cached().latest_angles.is_none());
    assert!(h.telemetry(0).samples.is_empty());
    let observed = Harness::new().adapter.capabilities();
    assert_eq!(
        observed.vehicles[0].link_loss_actions,
        vec![LinkLossPolicy::Neutralize]
    );
    assert!(targets(&h)?.is_empty());
    Ok(())
}

#[test]
fn capability_mapping_removes_yaw_and_preserves_diagonal_pitch() -> TestResult {
    let mut h = initialized()?;
    for millis in [100, 200] {
        let request = mapped_frame(&h, 0.0, 1.0)?;
        assert_eq!(h.apply(request, millis).disposition, Disposition::Accepted);
        h.step(millis);
    }
    assert!(targets(&h)?.is_empty());
    let request = mapped_frame(&h, -1.0, 1.0)?;
    assert_eq!(h.apply(request, 300).disposition, Disposition::Transformed);
    h.step(300);
    assert_eq!(targets(&h)?, [-5]);
    let request = mapped_frame(&h, 0.0, -1.0)?;
    assert_eq!(h.apply(request, 400).disposition, Disposition::Accepted);
    h.step(400);
    assert_eq!(targets(&h)?, [-5]);
    assert!(h.telemetry(400).samples.is_empty());
    Ok(())
}

#[test]
fn fractional_integration_is_independent_of_input_frequency() -> TestResult {
    for interval in [5, 10, 20, 25, 50, 100] {
        let mut h = initialized()?;
        for millis in (interval..=1000).step_by(interval as usize) {
            h.apply(frame(-BENCH_PITCH_RATE_LIMIT_RPS / 2.0), millis);
            h.step(millis);
        }
        assert_eq!(
            h.adapter.status_cached().commanded_pitch_tenths,
            Some(-25),
            "interval {interval}"
        );
    }
    Ok(())
}

#[test]
fn normal_io_scheduling_does_not_discard_input_time() -> TestResult {
    for input_interval in [20, 33, 50, 100] {
        let mut h = initialized()?;
        for millis in 1..=3300 {
            if millis % input_interval == 0 {
                h.apply(frame(-BENCH_PITCH_RATE_LIMIT_RPS), millis);
            }
            if millis % 10 == 0 {
                h.step(millis);
            }
        }
        let commanded = h
            .adapter
            .status_cached()
            .commanded_pitch_tenths
            .ok_or("missing target")?;
        assert!(
            (-165..=-164).contains(&commanded),
            "interval {input_interval}: {commanded}"
        );
    }
    Ok(())
}

#[test]
fn bench_reaches_nadir_but_never_commands_above_horizontal() -> TestResult {
    let mut h = initialized()?;
    for millis in (100..=19_000).step_by(100) {
        h.apply(frame(-BENCH_PITCH_RATE_LIMIT_RPS), millis);
        h.step(millis);
    }
    assert_eq!(h.adapter.status_cached().commanded_pitch_tenths, Some(-900));
    for millis in (19_100..=38_000).step_by(100) {
        h.apply(frame(BENCH_PITCH_RATE_LIMIT_RPS), millis);
        h.step(millis);
    }
    assert_eq!(h.adapter.status_cached().commanded_pitch_tenths, Some(0));
    assert!(
        targets(&h)?
            .windows(2)
            .all(|pair| (pair[0] - pair[1]).abs() <= 5)
    );
    assert!(h.telemetry(38_000).samples.is_empty());
    Ok(())
}

#[test]
fn recenter_slews_with_live_input_and_manual_pitch_cancels_it() -> TestResult {
    let mut h = initialized()?;
    for millis in (100..=3000).step_by(100) {
        h.apply(frame(-BENCH_PITCH_RATE_LIMIT_RPS), millis);
        h.step(millis);
    }
    assert_eq!(h.adapter.status_cached().commanded_pitch_tenths, Some(-150));
    let mut recenter = frame(0.0);
    recenter.actions.push(ControlAction::GimbalRecenter);
    let applied = h.apply(recenter.clone(), 3100);
    assert_eq!(applied.disposition, Disposition::Transformed);
    assert!(applied.action_results[0].accepted);
    h.step(3100);
    assert_eq!(h.adapter.status_cached().commanded_pitch_tenths, Some(-145));
    h.apply(frame(0.0), 3200);
    h.step(3200);
    assert_eq!(h.adapter.status_cached().commanded_pitch_tenths, Some(-140));
    h.apply(frame(-BENCH_PITCH_RATE_LIMIT_RPS), 3300);
    h.step(3300);
    h.apply(frame(0.0), 3400);
    h.step(3400);
    assert_eq!(h.adapter.status_cached().commanded_pitch_tenths, Some(-145));
    h.apply(recenter, 3500);
    h.step(3500);
    for millis in (3600..=6500).step_by(100) {
        h.apply(frame(0.0), millis);
        h.step(millis);
    }
    assert_eq!(h.adapter.status_cached().commanded_pitch_tenths, Some(0));
    assert!(
        targets(&h)?
            .windows(2)
            .all(|pair| (pair[0] - pair[1]).abs() <= 5)
    );
    Ok(())
}

#[test]
fn recenter_at_horizontal_does_not_follow_a_simultaneous_down_input() -> TestResult {
    let mut h = initialized()?;
    let mut request = frame(-BENCH_PITCH_RATE_LIMIT_RPS);
    request.actions.push(ControlAction::GimbalRecenter);
    h.apply(request, 100);
    h.step(100);
    assert!(targets(&h)?.is_empty());
    Ok(())
}

#[test]
fn long_gaps_and_rejected_inputs_do_not_add_catchup_motion() -> TestResult {
    let mut h = initialized()?;
    h.apply(frame(-BENCH_PITCH_RATE_LIMIT_RPS), 1000);
    h.step(1000);
    assert!(targets(&h)?.is_empty());
    h.apply(frame(-BENCH_PITCH_RATE_LIMIT_RPS), 1100);
    h.step(1100);
    assert_eq!(targets(&h)?, [-5]);
    assert!(matches!(
        h.apply(frame(f32::NAN), 1150).disposition,
        Disposition::Rejected(_)
    ));
    h.apply(frame(-BENCH_PITCH_RATE_LIMIT_RPS), 1190);
    h.step(1190);
    assert_eq!(h.adapter.status_cached().commanded_pitch_tenths, Some(-7));
    Ok(())
}

#[test]
fn pending_rejection_does_not_refresh_expiry_or_accumulate_demand() -> TestResult {
    let mut h = initialized()?;
    h.set_write_limit(0)?;
    h.apply(frame(-BENCH_PITCH_RATE_LIMIT_RPS), 100);
    h.step(100);
    assert!(matches!(
        h.apply(frame(-BENCH_PITCH_RATE_LIMIT_RPS), 150).disposition,
        Disposition::Rejected(_)
    ));
    h.set_write_limit(usize::MAX)?;
    h.step(199);
    assert_eq!(targets(&h)?, [-5]);
    h.apply(frame(-BENCH_PITCH_RATE_LIMIT_RPS), 200);
    h.step(200);
    assert_eq!(targets(&h)?, [-5]);
    h.apply(frame(-BENCH_PITCH_RATE_LIMIT_RPS), 219);
    h.step(219);
    assert_eq!(targets(&h)?, [-5, -6]);
    h.set_write_limit(0)?;
    h.apply(frame(-BENCH_PITCH_RATE_LIMIT_RPS), 239);
    h.step(239);
    assert!(matches!(
        h.apply(frame(-BENCH_PITCH_RATE_LIMIT_RPS), 300).disposition,
        Disposition::Rejected(_)
    ));
    h.set_write_limit(usize::MAX)?;
    h.step(339);
    assert_eq!(targets(&h)?, [-5, -6]);
    assert_eq!(h.adapter.status_cached().pending_bytes, 0);
    Ok(())
}

#[test]
fn partial_writes_commit_one_target_and_do_not_accumulate_wait_time() -> TestResult {
    let mut h = initialized()?;
    h.set_write_limit(7)?;
    h.apply(frame(-BENCH_PITCH_RATE_LIMIT_RPS), 100);
    for millis in [100, 130] {
        h.step(millis);
        assert_eq!(h.adapter.status_cached().commanded_pitch_tenths, Some(0));
    }
    h.step(160);
    assert_eq!(h.adapter.status_cached().commanded_pitch_tenths, Some(-5));
    assert_eq!(h.recorded("tx")?, h.written()?);
    h.set_write_limit(usize::MAX)?;
    h.apply(frame(-BENCH_PITCH_RATE_LIMIT_RPS), 180);
    h.step(180);
    assert_eq!(targets(&h)?, [-5, -6]);
    Ok(())
}

#[test]
fn serial_or_framing_faults_permanently_lock_bench_control() -> TestResult {
    for disconnected in [false, true] {
        let mut h = initialized()?;
        h.apply(frame(-BENCH_PITCH_RATE_LIMIT_RPS), 100);
        if disconnected {
            h.disconnect()?;
        } else {
            h.feed(&[0xff; 20])?;
        }
        h.step(100);
        assert!(h.adapter.status_cached().control_locked);
        assert!(h.link_loss(None, 120).is_err());
        assert!(matches!(
            h.apply(frame(-BENCH_PITCH_RATE_LIMIT_RPS), 120).disposition,
            Disposition::Rejected(_)
        ));
        h.step(120);
        assert!(targets(&h)?.is_empty());
    }
    Ok(())
}

#[test]
fn blocked_recording_time_is_excluded_but_subwire_fraction_is_retained() -> TestResult {
    let mut h = initialized()?;
    h.apply(frame(-BENCH_PITCH_RATE_LIMIT_RPS / 2.0), 10);
    h.feed(include_bytes!("../../duml/fixtures/c5-cdc-passive.bin"))?;
    let mut clock = [10, 90, 90, 90].into_iter();
    h.adapter
        .step_with_time_blocking(|| Duration::from_millis(clock.next().unwrap()));
    h.apply(frame(-BENCH_PITCH_RATE_LIMIT_RPS / 2.0), 100);
    h.step(100);
    assert_eq!(targets(&h)?, [-1]);
    Ok(())
}

#[test]
fn successful_write_commits_only_after_record_and_excludes_record_delay() -> TestResult {
    let mut h = initialized()?;
    h.apply(frame(-BENCH_PITCH_RATE_LIMIT_RPS), 100);
    assert_eq!(h.adapter.status_cached().commanded_pitch_tenths, Some(0));
    let mut clock = [100, 100, 130, 180, 180].into_iter();
    h.adapter
        .step_with_time_blocking(|| Duration::from_millis(clock.next().unwrap()));
    assert_eq!(h.adapter.status_cached().commanded_pitch_tenths, Some(-5));
    let progress = h.adapter.connection.execution.wire_progress().unwrap();
    assert_eq!(progress.prepared_at.unwrap().nanos, 100_000_000);
    assert_eq!(progress.sent_at.unwrap().nanos, 130_000_000);
    h.apply(frame(-BENCH_PITCH_RATE_LIMIT_RPS), 200);
    h.step(200);
    assert_eq!(targets(&h)?, [-5, -6]);
    assert_eq!(h.written()?, h.recorded("tx")?);
    Ok(())
}
