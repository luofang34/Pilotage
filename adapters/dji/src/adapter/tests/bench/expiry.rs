use std::time::Duration;

use pilotage_adapter_api::{Disposition, LinkLossPolicy, RejectReason};
use pilotage_protocol::ControlAction;

use super::super::{
    TestResult,
    harness::{Harness, frame},
};
use super::{initialized, targets};
use crate::adapter::{DjiFault, capability::MAX_PITCH_RATE};

#[test]
fn neutral_input_after_a_gap_cannot_resume_a_recenter() -> TestResult {
    let mut h = initialized()?;
    for millis in [100, 200, 300] {
        h.apply(frame(-MAX_PITCH_RATE), millis);
        h.step(millis);
    }
    let mut recenter = frame(0.0);
    recenter.actions.push(ControlAction::GimbalRecenter);
    h.apply(recenter.clone(), 400);
    h.step(400);
    assert_eq!(h.adapter.status_cached().commanded_pitch_tenths, Some(-10));
    let written = h.written()?;
    for millis in [600, 700] {
        h.apply(frame(0.0), millis);
        h.step(millis);
    }
    assert_eq!(h.written()?, written);
    h.apply(recenter, 800);
    h.step(800);
    assert_eq!(h.adapter.status_cached().commanded_pitch_tenths, Some(-5));
    Ok(())
}

#[test]
fn pending_pitch_is_writable_before_but_not_at_its_deadline() -> TestResult {
    for (write_at, expected) in [(119, vec![-1]), (120, Vec::new())] {
        let mut h = initialized()?;
        h.set_write_limit(0)?;
        assert_eq!(
            h.apply(frame(-MAX_PITCH_RATE), 20).disposition,
            Disposition::Transformed
        );
        h.step(20);
        assert_eq!(h.adapter.status_cached().pending_bytes, 21);
        assert_eq!(h.adapter.status_cached().commanded_pitch_tenths, Some(0));
        h.set_write_limit(usize::MAX)?;
        h.step(write_at);
        assert_eq!(targets(&h)?, expected);
        assert_eq!(h.adapter.status_cached().pending_bytes, 0);
        assert!(h.adapter.status_cached().fault.is_none());
        assert_eq!(
            h.adapter.status_cached().commanded_pitch_tenths,
            Some(if write_at < 120 { -1 } else { 0 })
        );
    }
    Ok(())
}

#[test]
fn expiry_of_a_partial_command_closes_the_connection() -> TestResult {
    let mut h = initialized()?;
    h.set_write_limit(3)?;
    h.apply(frame(-MAX_PITCH_RATE), 20);
    h.step(30);
    assert_eq!(h.adapter.status_cached().pending_bytes, 18);
    let partial = h.written()?;
    h.set_write_limit(usize::MAX)?;
    h.step(120);
    assert!(matches!(
        h.adapter.status_cached().fault,
        Some(DjiFault::PartialCommand)
    ));
    assert_eq!(h.adapter.status_cached().commanded_pitch_tenths, Some(0));
    assert_eq!(h.adapter.status_cached().pending_bytes, 0);
    assert!(h.adapter.status_cached().control_locked);
    assert!(h.link_loss(None, 120).is_err());
    assert!(matches!(
        h.apply(frame(MAX_PITCH_RATE), 140).disposition,
        Disposition::Rejected(_)
    ));
    h.step(140);
    assert_eq!(h.written()?, partial);
    assert_eq!(h.recorded("tx")?, partial);
    Ok(())
}

#[test]
fn receive_and_capture_time_cannot_extend_a_pending_command_deadline() -> TestResult {
    let traffic = include_bytes!("../../../duml/fixtures/c5-cdc-passive.bin");
    for partial in [false, true] {
        let mut h = initialized()?;
        h.apply(frame(-MAX_PITCH_RATE), 20);
        if partial {
            h.set_write_limit(3)?;
            h.step(30);
        }
        assert_eq!(
            h.adapter.status_cached().pending_bytes,
            if partial { 18 } else { 21 }
        );
        let before = h.written()?;
        let mut recorded_rx = h.recorded("rx")?;
        recorded_rx.extend(traffic);
        h.set_write_limit(usize::MAX)?;
        h.feed(traffic)?;
        let received_before = h.adapter.status_cached().received_bytes;
        let mut clock = [100, 3000, 3000].into_iter();
        h.adapter.step_with_time_blocking(|| {
            Duration::from_millis(clock.next().expect("step clock sample"))
        });
        assert_eq!(clock.next(), None);
        assert_eq!(h.recorded("rx")?, recorded_rx);
        assert_eq!(
            h.adapter.status_cached().received_bytes,
            received_before + traffic.len() as u64
        );
        assert_eq!(h.written()?, before);
        assert_eq!(h.adapter.status_cached().pending_bytes, 0);
        assert_eq!(h.adapter.status_cached().commanded_pitch_tenths, Some(0));
        if partial {
            assert!(matches!(
                h.adapter.status_cached().fault,
                Some(DjiFault::PartialCommand)
            ));
            assert!(h.adapter.status_cached().control_locked);
        } else {
            assert!(h.adapter.status_cached().fault.is_none());
            assert!(targets(&h)?.is_empty());
        }
    }
    Ok(())
}

#[test]
fn reverse_or_neutral_input_discards_an_unsent_old_direction() -> TestResult {
    for rate in [MAX_PITCH_RATE, 0.0] {
        let mut h = initialized()?;
        h.apply(frame(-MAX_PITCH_RATE), 100);
        h.step(100);
        assert_eq!(targets(&h)?, vec![-5]);
        h.set_write_limit(0)?;
        h.apply(frame(-MAX_PITCH_RATE), 120);
        h.step(120);
        assert_eq!(h.adapter.status_cached().pending_bytes, 21);
        assert!(matches!(
            h.apply(frame(rate), 130).disposition,
            Disposition::Accepted | Disposition::Transformed
        ));
        assert_eq!(h.adapter.status_cached().pending_bytes, 0);
        h.set_write_limit(usize::MAX)?;
        h.step(130);
        assert_eq!(targets(&h)?, vec![-5]);
        h.apply(frame(rate), 150);
        h.step(150);
        assert_eq!(
            targets(&h)?,
            if rate == 0.0 { vec![-5] } else { vec![-5, -4] }
        );
        assert!(h.adapter.status_cached().fault.is_none());
    }
    Ok(())
}

#[test]
fn lease_loss_permanently_locks_a_bench_attachment() -> TestResult {
    let mut h = initialized()?;
    h.link_loss(None, 0)?;
    h.apply(frame(-MAX_PITCH_RATE), 100);
    assert_eq!(h.adapter.status_cached().pending_bytes, 21);
    assert!(h.link_loss(Some(LinkLossPolicy::Neutralize), 100).is_err());
    assert_eq!(h.adapter.status_cached().pending_bytes, 0);
    assert!(h.adapter.status_cached().control_locked);
    for millis in [100, 120, 3000] {
        assert!(h.link_loss(None, millis).is_err());
        assert_eq!(
            h.apply(frame(-MAX_PITCH_RATE), millis).disposition,
            Disposition::Rejected(RejectReason::LinkLossEngaged)
        );
        h.step(millis);
    }
    assert!(targets(&h)?.is_empty());
    assert_eq!(h.adapter.status_cached().commanded_pitch_tenths, Some(0));
    Ok(())
}

#[test]
fn capture_failure_after_a_full_serial_write_does_not_commit_the_target() -> TestResult {
    let mut h: Harness = initialized()?;
    let recorded = h.recorded("tx")?;
    h.apply(frame(-MAX_PITCH_RATE), 100);
    h.fail_capture()?;
    h.step(100);
    assert_eq!(targets(&h)?, vec![-5]);
    assert_eq!(h.recorded("tx")?, recorded);
    assert_eq!(h.adapter.status_cached().commanded_pitch_tenths, Some(0));
    assert!(matches!(
        h.adapter.status_cached().fault,
        Some(DjiFault::Recording { .. })
    ));
    assert!(h.adapter.status_cached().control_locked);
    assert!(h.link_loss(None, 100).is_err());
    let written = h.written()?;
    assert!(matches!(
        h.apply(frame(-MAX_PITCH_RATE), 120).disposition,
        Disposition::Rejected(_)
    ));
    h.step(120);
    assert_eq!(h.written()?, written);
    Ok(())
}
