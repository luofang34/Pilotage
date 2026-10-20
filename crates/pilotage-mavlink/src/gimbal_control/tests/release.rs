use super::*;

fn release_command(commands: &mut mpsc::Receiver<OutboundCommand>) -> OutboundCommand {
    while let Ok(command) = commands.try_recv() {
        if command.command == 1001 {
            return command;
        }
    }
    panic!("expected manager release");
}

fn complete_write(command: &OutboundCommand) -> bool {
    let sent = command
        .fence
        .as_ref()
        .unwrap()
        .with_valid_write(|| Ok(true))
        .unwrap();
    if sent {
        command
            .receipt
            .as_ref()
            .unwrap()
            .record_sent(Instant::now());
    }
    sent
}

#[test]
fn unsent_motion_releases_owned_manager_without_inventing_a_stop() {
    let (mut control, mut commands, rates) = control();
    owned(&mut control);
    assert!(control.rate_demand(0.5, 0.0));
    let pending = rates.borrow().clone().unwrap();
    control.invalidate_pending();
    control.maintain();
    assert!(!pending.fence.unwrap().is_valid());
    assert!(rates.borrow().is_none());
    let release = release_command(&mut commands);
    assert_eq!((release.target_component, release.params[6]), (1, 2.0));
    assert_eq!(release.params[0], -3.0);
    assert!(!control.rate_demand(0.5, 0.0));
    control.invalidate_pending();
    assert!(complete_write(&release));
    control.maintain();
    assert!(control.ownership_release.is_none());
}

#[test]
fn a_sent_claim_is_released_even_if_unowned_status_arrives_after_revoke() {
    let (mut control, mut commands, rates) = control();
    observe(&mut control, status(1, 2, (0, 0)));
    assert!(!control.rate_demand(0.5, 0.0));
    let claim = release_command(&mut commands);
    assert_eq!(claim.params[0], f32::from(GCS_SYSTEM_ID));
    assert!(complete_write(&claim));
    control.invalidate_pending();
    observe(&mut control, status(2, 2, (0, 0)));
    control.maintain();
    assert!(control.ownership_release.is_some());
    let release = release_command(&mut commands);
    assert_eq!(release.params[0], -3.0);
    assert!(rates.borrow().is_none());
    assert!(complete_write(&release));
    control.maintain();
    assert!(control.ownership_release.is_none());
}

#[test]
fn discovering_an_owned_manager_does_not_release_it_without_a_local_request() {
    let (mut control, mut commands, rates) = control();
    owned(&mut control);
    control.invalidate_pending();
    control.maintain();
    while let Ok(command) = commands.try_recv() {
        assert_eq!(command.command, 512);
    }
    assert!(rates.borrow().is_none());
    assert!(control.ownership_release.is_none());
}

#[test]
fn a_refused_release_queue_keeps_the_obligation_until_a_complete_write() {
    let (mut control, mut commands, rates) = control();
    owned(&mut control);
    assert!(control.rate_demand(0.5, 0.0));
    let status_request = commands.try_recv().unwrap();
    for _ in 0..16 {
        control.commands.try_send(status_request.clone()).unwrap();
    }
    assert!(!control.queue_link_loss_stop());
    assert!(control.ownership_release.is_some());
    assert!(!control.rate_demand(0.5, 0.0));
    while commands.try_recv().is_ok() {}
    control.maintain();
    let release = release_command(&mut commands);
    assert!(complete_write(&release));
    control.maintain();
    assert!(control.ownership_release.is_none());
    assert!(rates.borrow().is_none());
}

#[test]
fn a_new_owner_fences_an_already_queued_release() {
    let (mut control, mut commands, _rates) = control();
    owned(&mut control);
    assert!(control.rate_demand(0.5, 0.0));
    control.invalidate_pending();
    control.maintain();
    let release = release_command(&mut commands);
    observe(&mut control, status(2, 2, (42, 190)));
    assert!(!complete_write(&release));
}

#[test]
fn a_capability_change_keeps_release_bound_to_the_same_attachment() {
    let (mut control, mut commands, rates) = control();
    owned(&mut control);
    assert!(control.rate_demand(0.5, 0.0));
    control.invalidate_pending();
    control.maintain();
    let old_release = release_command(&mut commands);
    let mut information = control
        .manager_state
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .gimbals
        .managers
        .get(&(1, 2))
        .unwrap()
        .information
        .unwrap()
        .0;
    information.time_boot_ms = 2;
    information.pitch_max = 1.0;
    crate::link::apply_messages_at(
        control.manager_state.as_ref().unwrap(),
        &[(
            crate::codec::FrameSource {
                system_id: 1,
                component_id: 1,
                frame_sequence: 1,
            },
            crate::codec::FcMessage::GimbalManagerInformation(information),
        )],
        0,
        0,
        Instant::now(),
    );
    control.maintain();
    assert!(!complete_write(&old_release));
    let release = release_command(&mut commands);
    assert!(complete_write(&release));
    control.maintain();
    assert!(control.ownership_release.is_none());
    assert!(rates.borrow().is_none());
}

#[test]
fn capability_change_before_revoke_keeps_existing_ownership_responsibility() {
    let (mut control, mut commands, rates) = control();
    owned(&mut control);
    assert!(control.rate_demand(0.5, 0.0));
    let mut information = control
        .manager_state
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .gimbals
        .managers
        .get(&(1, 2))
        .unwrap()
        .information
        .unwrap()
        .0;
    information.time_boot_ms = 2;
    information.pitch_max = 1.0;
    crate::link::apply_messages_at(
        control.manager_state.as_ref().unwrap(),
        &[(
            crate::codec::FrameSource {
                system_id: 1,
                component_id: 1,
                frame_sequence: 1,
            },
            crate::codec::FcMessage::GimbalManagerInformation(information),
        )],
        0,
        0,
        Instant::now(),
    );
    control.refresh_discovery();
    control.invalidate_pending();
    control.maintain();
    let release = release_command(&mut commands);
    assert_eq!(release.params[0], -3.0);
    assert!(complete_write(&release));
    assert!(rates.borrow().is_none());
}
