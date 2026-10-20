use super::*;

#[test]
fn only_a_new_independent_report_after_a_complete_write_can_confirm_mode() {
    let mut h = ready();
    request(&mut h, 11, 100).unwrap();
    ack(&mut h, VirtualStickMode::Enable, 2, 0);
    h.step(12);
    assert_eq!(status(&h).enable.acknowledgement, None);
    for sequence in [10, 9] {
        report(&mut h, sequence, true, true, false);
        h.step(13);
    }
    ack(&mut h, VirtualStickMode::Enable, 99, 0);
    h.step(14);
    assert_eq!(status(&h).enable.observed_at, None);
    assert_eq!(status(&h).enable.acknowledgement, None);
    ack(&mut h, VirtualStickMode::Enable, 2, 0);
    report(&mut h, 11, true, true, false);
    h.step(15);
    assert_eq!(status(&h).phase, Phase::Disable);
    assert_eq!(commands(&h).len(), 2);
}

#[test]
fn first_ack_decision_is_immutable_and_rejection_does_not_suppress_cleanup() {
    for codes in [[0, 7], [7, 0]] {
        let mut h = ready();
        request(&mut h, 11, 100).unwrap();
        h.step(12);
        for code in codes {
            ack(&mut h, VirtualStickMode::Enable, 2, code);
        }
        report(&mut h, 11, true, true, false);
        h.step(13);
        assert_eq!(status(&h).enable.acknowledgement, Some(codes[0]));
        assert_eq!(status(&h).phase, Phase::Disable);
        assert_eq!(commands(&h).len(), 2);
        assert_eq!(status(&h).reason.is_some(), codes[0] != 0);
    }
}

#[test]
fn enable_timeout_has_one_fixed_cleanup_attempt_and_no_velocity_or_enable_retry() {
    let mut h = ready();
    request(&mut h, 11, 100).unwrap();
    h.step(12);
    for (sequence, time) in [
        (2, 250),
        (3, 500),
        (4, 750),
        (5, 1000),
        (6, 1250),
        (7, 1350),
    ] {
        feed_fc(&mut h, sequence, 0);
        h.step(time);
    }
    assert_eq!(status(&h).reason, Some(Reason::EnableUnconfirmed));
    assert_eq!(status(&h).phase, Phase::Disable);
    assert_eq!(
        commands(&h)
            .iter()
            .map(|frame| frame.payload()[0])
            .collect::<Vec<_>>(),
        [1, 2]
    );
    for sequence in 8..=14 {
        feed_fc(&mut h, sequence, 0);
        h.step(1350 + u64::from(sequence - 7) * 250);
    }
    assert_eq!(status(&h).phase, Phase::Finished);
    assert_eq!(status(&h).disable.observed_at, None);
    assert_eq!(commands(&h).len(), 2);
    assert!(request(&mut h, 3110, 3200).is_err());
}

#[test]
fn disabled_feedback_without_ack_does_not_claim_the_diagnostic_closed() {
    let mut h = ready();
    request(&mut h, 11, 100).unwrap();
    h.step(12);
    ack(&mut h, VirtualStickMode::Enable, 2, 0);
    report(&mut h, 11, true, true, false);
    h.step(13);
    report(&mut h, 12, false, true, false);
    h.step(14);
    assert_eq!(status(&h).phase, Phase::Disable);
    assert!(status(&h).disable.observed_at.is_some());
    assert_eq!(status(&h).disable.acknowledgement, None);
}

#[test]
fn repeated_mode_samples_cannot_renew_configuration_freshness() {
    let mut h = ready();
    for (sequence, time) in [
        (2, 250),
        (3, 500),
        (4, 750),
        (5, 1000),
        (6, 1250),
        (7, 1500),
        (8, 1512),
    ] {
        feed_fc(&mut h, sequence, 0);
        report(&mut h, 10, false, true, false);
        h.step(time);
    }
    assert_eq!(request(&mut h, 1512, 1600), Err(Error::MissingFeedback));
    assert!(commands(&h).is_empty());
}

#[test]
fn terminal_cleanup_evidence_never_converts_to_flight_or_motor_stop_completion() {
    let mut h = ready();
    request(&mut h, 11, 100).unwrap();
    h.adapter.cancel_virtual_stick_diagnostic();
    h.step(12);
    assert_eq!(status(&h).phase, Phase::Finished);
    assert_eq!(status(&h).reason, Some(Reason::Cancelled));
    assert!(commands(&h).is_empty());
    assert!(
        h.adapter
            .sample_telemetry()
            .samples
            .iter()
            .all(|sample| sample.flight_procedure.is_none())
    );
}

#[test]
fn malformed_forward_mode_report_with_valid_crc_withholds_configuration() {
    let mut h = ready();
    let mut bytes = vec![0x55, 90, 4, 0, 14, 2, 11, 0, 0, 3, 0x44];
    bytes.extend([0; 79]);
    seal_duml(&mut bytes);
    feed(&mut h, &bytes);
    h.step(12);
    assert_eq!(request(&mut h, 12, 100), Err(Error::MissingFeedback));
    assert!(commands(&h).is_empty());
}

#[test]
fn independent_one_hertz_mode_feedback_finishes_inside_the_original_fixed_budget() {
    let mut h = ready();
    request(&mut h, 11, 100).unwrap();
    h.step(12);
    ack(&mut h, VirtualStickMode::Enable, 2, 0);
    h.step(13);
    for sequence in 2..=5 {
        feed_fc(&mut h, sequence, 0);
        if sequence == 5 {
            report(&mut h, 11, true, true, false);
        }
        h.step(u64::from(sequence - 1) * 250);
    }
    assert_eq!(status(&h).phase, Phase::Disable);
    ack(&mut h, VirtualStickMode::Disable, 3, 0);
    h.step(1001);
    for sequence in 6..=9 {
        feed_fc(&mut h, sequence, 0);
        if sequence == 9 {
            report(&mut h, 12, false, true, false);
        }
        h.step(u64::from(sequence - 1) * 250);
    }
    let result = status(&h);
    assert_eq!(result.phase, Phase::Finished);
    assert_eq!(result.reason, None);
    assert_eq!(result.enable.observed_sequence, Some(11));
    assert_eq!(result.disable.observed_sequence, Some(12));
    assert_eq!(result.enable.observed_at, Some(Duration::from_secs(1)));
    assert_eq!(result.disable.observed_at, Some(Duration::from_secs(2)));
    assert_eq!(result.enable_send_until, Duration::from_millis(100));
    assert_eq!(result.enable_observe_until, Duration::from_millis(1350));
    assert_eq!(result.disable_send_until, Duration::from_millis(1600));
    assert_eq!(result.complete_until, Duration::from_millis(3100));
    assert_eq!(commands(&h).len(), 2);
}
