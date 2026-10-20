use super::*;
use pilotage_adapter_api::VehicleAdapter;
use pilotage_protocol::{ScopeId, VehicleId};

#[test]
fn existing_configuration_and_fresh_stopped_motor_battery_are_required_without_writes() {
    for (enabled, multiple, novice) in [
        (true, true, false),
        (false, false, false),
        (false, true, true),
    ] {
        let mut h = ready();
        report(&mut h, 11, enabled, multiple, novice);
        h.step(12);
        assert_eq!(request(&mut h, 12, 100), Err(Error::Configuration));
        assert!(commands(&h).is_empty());
    }
    for (motor, battery) in [(8, 50), (12, 50), (0, 24)] {
        let mut h = ready();
        feed_fc(&mut h, 2, motor);
        feed_battery(&mut h, 2, battery);
        h.step(12);
        assert_eq!(request(&mut h, 12, 100), Err(Error::MissingFeedback));
        assert!(commands(&h).is_empty());
    }
}

#[test]
fn queue_preserves_original_deadline_and_rechecks_motor_battery_and_model() {
    for case in 0..4 {
        let mut h = ready();
        request(&mut h, 11, 100).unwrap();
        match case {
            0 => feed_fc(&mut h, 2, 8),
            1 => feed_battery(&mut h, 2, 24),
            2 => h.feed_model(88, 2).unwrap(),
            _ => {}
        }
        h.step(if case == 3 { 100 } else { 12 });
        assert!(commands(&h).is_empty());
        assert_eq!(status(&h).phase, Phase::Finished);
        assert!(!status(&h).enable.write_started);
    }
}

#[test]
fn expiry_and_same_batch_model_recovery_cannot_restore_a_queued_probe() {
    let mut h = ready();
    request(&mut h, 11, 250).unwrap();
    h.set_write_limit(0).unwrap();
    h.step(12);
    h.feed_model(87, 2).unwrap();
    h.set_write_limit(usize::MAX).unwrap();
    h.step(311);
    assert!(commands(&h).is_empty());
    assert_eq!(status(&h).reason, Some(Reason::SourceChanged));
}

#[test]
fn partial_write_is_never_followed_by_disable_or_another_command() {
    let mut h = ready();
    request(&mut h, 11, 100).unwrap();
    h.set_write_limit(3).unwrap();
    h.step(12);
    let written = h.written().unwrap();
    assert!(status(&h).enable.write_started);
    assert_eq!(status(&h).enable.written_at, None);
    h.adapter.cancel_virtual_stick_diagnostic();
    h.set_write_limit(usize::MAX).unwrap();
    h.step(13);
    assert!(h.adapter.status_cached().fault.is_some());
    assert_eq!(h.written().unwrap(), written);
    assert_eq!(status(&h).reason, Some(Reason::TransportUncertain));
}

#[test]
fn unconfirmed_transport_or_capture_receipt_never_enables_automatic_retry() {
    for capture in [false, true] {
        let mut h = ready();
        request(&mut h, 11, 100).unwrap();
        if capture {
            h.fail_capture().unwrap();
        } else {
            h.lose_write_receipt().unwrap();
        }
        h.step(12);
        assert_eq!(status(&h).phase, Phase::Finished);
        assert_eq!(status(&h).reason, Some(Reason::TransportUncertain));
        assert!(status(&h).enable.write_started);
        assert_eq!(status(&h).enable.written_at, None);
        let before = h.written().unwrap();
        h.step(13);
        assert_eq!(h.written().unwrap(), before);
        assert!(request(&mut h, 14, 100).is_err());
    }
}

#[test]
fn current_scope_invalidation_requests_one_cleanup_without_renewing_its_budget() {
    let mut h = ready();
    request(&mut h, 11, 100).unwrap();
    h.step(12);
    h.adapter
        .invalidate_pending_commands(VehicleId::new(1), &ScopeId::new("vehicle.motion"));
    h.step(13);
    assert_eq!(
        commands(&h)
            .iter()
            .map(|frame| frame.payload()[0])
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(status(&h).reason, Some(Reason::Cancelled));
    for sequence in 2..=14 {
        feed_fc(&mut h, sequence, 0);
        h.step(13 + u64::from(sequence - 1) * 250);
    }
    assert_eq!(status(&h).phase, Phase::Finished);
    assert_eq!(commands(&h).len(), 2);
    assert_eq!(status(&h).disable.acknowledgement, None);
}

#[test]
fn a_late_full_write_is_recorded_but_not_treated_as_a_valid_mode_exchange() {
    let mut h = ready();
    request(&mut h, 11, 100).unwrap();
    let mut ticks = [12, 12, 101].into_iter();
    h.adapter
        .step_with_time_blocking(|| Duration::from_millis(ticks.next().unwrap_or(101)));
    assert_eq!(commands(&h).len(), 1);
    assert!(status(&h).enable.write_started);
    assert_eq!(
        status(&h).enable.written_at,
        Some(Duration::from_millis(101))
    );
    assert_eq!(status(&h).reason, Some(Reason::TransportUncertain));
    assert!(h.adapter.status_cached().fault.is_some());
}

#[test]
fn a_same_batch_model_replacement_and_restore_cannot_accept_an_old_mode_ack() {
    let mut h = ready();
    request(&mut h, 11, 100).unwrap();
    h.step(12);
    h.feed_model(88, 2).unwrap();
    h.feed_model(87, 3).unwrap();
    ack(&mut h, VirtualStickMode::Enable, 2, 0);
    report(&mut h, 11, true, true, false);
    h.step(13);
    assert_eq!(status(&h).phase, Phase::Finished);
    assert_eq!(status(&h).reason, Some(Reason::SourceChanged));
    assert_eq!(status(&h).enable.acknowledgement, None);
    assert_eq!(commands(&h).len(), 1);
}

#[test]
fn link_loss_requests_cleanup_even_when_the_gimbal_hold_cannot_be_verified() {
    let mut h = ready();
    request(&mut h, 11, 100).unwrap();
    h.step(12);
    assert!(
        h.link_loss(Some(pilotage_adapter_api::LinkLossPolicy::Neutralize), 13)
            .is_err()
    );
    h.step(14);
    assert_eq!(
        commands(&h)
            .iter()
            .map(|frame| frame.payload()[0])
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(status(&h).reason, Some(Reason::Cancelled));
}
