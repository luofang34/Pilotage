use super::*;

fn pitch(value: f32) -> PayloadCommand {
    PayloadCommand::GimbalRate(turret_core::GimbalRate {
        rates: turret_core::Axes {
            pitch: Some(AngularRate::radians_per_second(value).unwrap()),
            roll: None,
            yaw: None,
        },
        frame: turret_core::ReferenceFrame::HorizonVehicleYaw,
    })
}

#[test]
fn unsent_recenter_never_changes_the_axis_stopped_on_revocation() {
    let (mut control, _commands, rates) = control();
    owned(&mut control);
    assert!(control.command(pitch(0.4)));
    let active = rates.borrow().clone().unwrap();
    assert!(active.payload_write.unwrap().write(1, || Ok(1)).unwrap());
    control.bind_timing(None);
    assert!(control.neutral());
    assert!(control.queue_link_loss_stop());
    let stop = rates.borrow().clone().unwrap();
    assert_eq!(stop.pitch_rps, 0.0);
    assert!(stop.yaw_rps.is_nan());
    assert!(control.channel.poll_effect().unwrap().is_some());
}

#[test]
fn two_uncertain_actions_keep_separate_stops_and_release_only_after_both_writes() {
    let (mut control, mut commands, rates) = control();
    owned(&mut control);
    assert!(control.command(pitch(0.4)));
    let first = rates.borrow().clone().unwrap();
    assert!(first.payload_write.unwrap().write(1, || Ok(1)).unwrap());
    control.bind_timing(None);
    assert!(control.command(pitch(-0.4)));
    let attempted = rates.borrow().clone().unwrap();
    assert!(attempted.payload_write.unwrap().write(2, || Ok(1)).is_err());
    while commands.try_recv().is_ok() {}
    control.maintain();
    let first_stop = rates.borrow().clone().unwrap();
    let first_effect = control.release_after_stop.as_ref().unwrap().0;
    control.maintain();
    assert_eq!(control.release_after_stop.as_ref().unwrap().0, first_effect);
    assert!(commands.try_recv().is_err());
    first_stop.receipt.unwrap().record_sent(Instant::now());
    control.maintain();
    let second_stop = rates.borrow().clone().unwrap();
    let second_effect = control.release_after_stop.as_ref().unwrap().0;
    assert_ne!(
        first_effect.command.identity,
        second_effect.command.identity
    );
    assert_eq!(first_effect.command.payload, pitch(0.4));
    assert_eq!(second_effect.command.payload, pitch(-0.4));
    assert!(commands.try_recv().is_err());
    second_stop.receipt.unwrap().record_sent(Instant::now());
    control.maintain();
    assert!(control.channel.poll_effect().unwrap().is_none());
    let release = commands.try_recv().unwrap();
    assert_eq!(release.command, 1001);
    assert_eq!(release.params[0], -3.0);
}

#[test]
fn unclaimed_manager_does_not_discharge_a_refused_physical_stop() {
    let (mut control, _commands, rates) = control();
    owned(&mut control);
    assert!(control.command(pitch(0.4)));
    let first = rates.borrow().clone().unwrap();
    assert!(first.payload_write.unwrap().write(1, || Ok(1)).unwrap());
    drop(rates);
    assert!(!control.queue_link_loss_stop());
    let expected = control.channel.poll_effect().unwrap().unwrap();
    observe(&mut control, status(2, 2, (0, 0)));
    control.maintain();
    assert_eq!(control.channel.poll_effect().unwrap(), Some(expected));
    observe(&mut control, status(3, 2, (42, 190)));
    control.maintain();
    assert!(control.channel.poll_effect().unwrap().is_none());
}
