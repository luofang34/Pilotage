use super::super::flight::remote;
use super::*;
use pilotage_protocol::{ControlAction, Generation};

#[test]
fn ground_authority_is_revoked_by_remote_input_without_acquiring_virtual_stick() {
    let mut h = ready();
    h.adapter.experimental_virtual_stick = true;
    assert_eq!(input(&mut h, 1, 11, [0.0; 4]), Disposition::Accepted);
    remote(&mut h, 2, [1024, 1024, 955, 1024]);
    h.step(12);
    assert!(commands(&h).is_empty());
    assert_eq!(h.adapter.take_control_authority_losses().len(), 1);
    remote(&mut h, 3, [1024; 4]);
    h.step(13);
    assert_eq!(
        input(&mut h, 2, 14, [0.0; 4]),
        Disposition::Rejected(pilotage_adapter_api::RejectReason::Fenced)
    );
}

#[test]
fn zero_sequence_pushes_refresh_monitoring_and_revoke_on_a_deflection() {
    let mut h = enabled();
    for sequence in 3..=14 {
        let at = 20 + u64::from(sequence - 3) * 50;
        flying(&mut h, sequence);
        remote(&mut h, 0, [1024; 4]);
        assert_eq!(
            input(&mut h, u32::from(sequence), at, [0.0; 4]),
            Disposition::Accepted
        );
        h.step(at);
    }
    assert!(h.adapter.connection.mode_session.enabled());
    // The captured right-stick gesture changes channel one without a sequence change.
    remote(&mut h, 0, [1684, 1024, 1024, 1024]);
    remote(&mut h, 0, [1024; 4]);
    h.step(580);
    assert_eq!(h.adapter.take_control_authority_losses().len(), 1);
    assert_eq!(commands(&h).last().unwrap().payload(), [2]);
    assert!(matches!(
        input(&mut h, 15, 581, [0.0; 4]),
        Disposition::Rejected(pilotage_adapter_api::RejectReason::Fenced)
    ));
}

#[test]
fn native_mode_exit_revokes_authority_even_with_centered_remote_sticks() {
    let mut h = enabled();
    report(&mut h, 12, false, true, false);
    h.step(20);
    assert_eq!(h.adapter.take_control_authority_losses().len(), 1);
    assert_eq!(
        input(&mut h, 2, 21, [0.0; 4]),
        Disposition::Rejected(pilotage_adapter_api::RejectReason::Fenced)
    );
}

#[test]
fn physical_sticks_release_mode_and_cannot_be_reclaimed_by_the_old_lease() {
    let mut h = enabled();
    input(&mut h, 2, 20, [0.4, 0.0, 0.0, 0.0]);
    // RC-N1 channel values observed during attempted manual takeover.
    remote(&mut h, 2, [1024, 1024, 955, 1024]);
    remote(&mut h, 3, [1024; 4]);
    h.step(21);
    let losses = h.adapter.take_control_authority_losses();
    assert_eq!(losses.len(), 1);
    assert_eq!(losses[0].generation, Generation::new(1));
    assert!(h.adapter.take_control_authority_losses().is_empty());
    let release = commands(&h).pop().unwrap();
    assert_eq!((release.command_id(), release.payload()), (0x80, &[2][..]));
    ack(&mut h, VirtualStickMode::Disable, release.sequence(), 0);
    report(&mut h, 12, false, true, false);
    h.step(22);
    for (sequence, value) in [(3, [0.4, 0.0, 0.0, 0.0]), (4, [0.0; 4])] {
        assert_eq!(
            input(&mut h, sequence, 23, value),
            Disposition::Rejected(pilotage_adapter_api::RejectReason::Fenced)
        );
    }
    assert_eq!(
        h.apply(landing::land(), 24).disposition,
        Disposition::Rejected(pilotage_adapter_api::RejectReason::Fenced)
    );
    let mut takeoff = super::super::flight::request(ControlAction::Takeoff, 73);
    takeoff.generation = Generation::new(1);
    assert!(matches!(
        h.apply(takeoff, 24).disposition,
        Disposition::Rejected(_)
    ));
    let before = commands(&h).len();
    h.step(25);
    assert_eq!(commands(&h).len(), before);
}

#[test]
fn usb_loss_revokes_the_grant_without_claiming_a_mode_release() {
    let mut h = enabled();
    h.disconnect().unwrap();
    h.step(21);
    assert!(h.adapter.status_cached().fault.is_some());
    assert!(h.adapter.connection.mode_session.unresolved());
    let losses = h.adapter.take_control_authority_losses();
    assert_eq!(losses.len(), 1);
    assert_eq!(losses[0].generation, Generation::new(1));
    assert!(h.adapter.take_control_authority_losses().is_empty());
}

#[test]
fn remote_takeover_cannot_be_undone_by_landing_cancellation_resume() {
    let mut h = override_control::landing();
    input(&mut h, 2, 25, [0.0; 4]);
    input(&mut h, 3, 26, [0.4, 0.0, 0.0, 0.0]);
    remote(&mut h, 2, [1024, 1024, 364, 1024]);
    h.step(27);
    assert_eq!(commands(&h).last().unwrap().payload(), [2]);
    for (sequence, time) in [(4, 28), (5, 29), (6, 30)] {
        flying_mode(&mut h, sequence, 6);
        remote(&mut h, sequence, [1024; 4]);
        h.step(time);
    }
    assert_eq!(
        input(&mut h, 4, 31, [0.4, 0.0, 0.0, 0.0]),
        Disposition::Rejected(pilotage_adapter_api::RejectReason::Fenced)
    );
    h.step(32);
    assert_eq!(commands(&h).last().unwrap().payload(), [2]);
}

#[test]
fn stale_physical_stick_monitoring_releases_despite_fresh_aircraft_and_xbox_input() {
    let mut h = enabled();
    for sequence in 3..=12 {
        let at = 20 + u64::from(sequence - 3) * 50;
        flying(&mut h, sequence);
        assert_eq!(
            input(&mut h, u32::from(sequence), at, [0.4, 0.0, 0.0, 0.0]),
            Disposition::Accepted
        );
        h.step(at);
    }
    assert!(h.adapter.connection.mode_session.enabled());
    flying(&mut h, 13);
    report(&mut h, 12, true, true, false);
    // Duplicate neutral reports cannot refresh the monitoring deadline.
    remote(&mut h, 1, [1024; 4]);
    h.step(510);
    assert_eq!(commands(&h).last().unwrap().payload(), [2]);
    remote(&mut h, 2, [1024; 4]);
    h.step(511);
    assert!(matches!(
        input(&mut h, 13, 512, [0.0; 4]),
        Disposition::Rejected(_)
    ));
}

#[test]
fn physical_takeover_during_a_partial_write_cannot_claim_a_delivered_release() {
    let mut h = enabled();
    input(&mut h, 2, 20, [0.4, 0.0, 0.0, 0.0]);
    h.set_write_limit(3).unwrap();
    h.step(70);
    let before = h.written().unwrap();
    remote(&mut h, 2, [1024, 1024, 364, 1024]);
    h.set_write_limit(4096).unwrap();
    h.step(71);
    assert!(h.adapter.status_cached().fault.is_some());
    assert_eq!(h.written().unwrap(), before);
    assert!(h.adapter.connection.mode_session.unresolved());
}
