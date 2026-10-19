use super::*;
use pilotage_mavlink::codec::{GCS_COMPONENT_ID, GCS_SYSTEM_ID, GimbalManagerStatus};
use tokio::sync::mpsc;

fn control() -> (
    Px4GimbalControl,
    mpsc::Receiver<OutboundCommand>,
    watch::Receiver<Option<GimbalRateDemand>>,
) {
    let (commands, receiver) = mpsc::channel(16);
    let (rates, observer) = watch::channel(None);
    (
        Px4GimbalControl::new(commands, rates, 1, 1)
            .unwrap()
            .with_manager_state(std::sync::Arc::new(std::sync::Mutex::new(
                pilotage_mavlink::LinkState::default(),
            ))),
        receiver,
        observer,
    )
}
fn status(time: u32, device: u8, owner: (u8, u8)) -> GimbalManagerStatus {
    GimbalManagerStatus {
        time_boot_ms: time,
        flags: 0,
        device_id: device,
        primary_system: owner.0,
        primary_component: owner.1,
    }
}
fn observe(control: &mut Px4GimbalControl, report: GimbalManagerStatus) {
    let received = Instant::now();
    pilotage_mavlink::link::apply_messages_at(
        control.manager_state.as_ref().unwrap(),
        &[(
            pilotage_mavlink::codec::FrameSource {
                system_id: 1,
                component_id: 1,
                frame_sequence: 1,
            },
            pilotage_mavlink::codec::FcMessage::GimbalManagerStatus(report),
        )],
        0,
        0,
        received,
    );
    control.observe_manager(report, received);
}
fn owned(control: &mut Px4GimbalControl) {
    observe(control, status(1, 2, (GCS_SYSTEM_ID, GCS_COMPONENT_ID)));
}
#[test]
fn unknown_owner_requests_status_without_control_or_claim() {
    let (mut control, mut commands, rates) = control();
    assert!(!control.rate_demand(0.5, 0.0));
    assert_eq!(commands.try_recv().unwrap().command, 512);
    assert!(commands.try_recv().is_err());
    assert!(rates.borrow().is_none());
}
#[test]
fn free_manager_can_be_claimed_but_queueing_does_not_authorize_motion() {
    let (mut control, mut commands, rates) = control();
    observe(&mut control, status(1, 2, (0, 0)));
    assert!(!control.rate_demand(0.5, 0.0));
    assert_eq!(commands.try_recv().unwrap().command, 512);
    let claim = commands.try_recv().unwrap();
    assert_eq!(claim.command, 1001);
    assert_eq!(claim.params[6], 2.0);
    assert!(rates.borrow().is_none());
    observe(
        &mut control,
        status(2, 2, (GCS_SYSTEM_ID, GCS_COMPONENT_ID)),
    );
    assert!(control.rate_demand(0.5, -0.25));
    let demand = rates.borrow().clone().unwrap();
    assert_eq!(demand.gimbal_device_id, 2);
    assert_eq!((demand.pitch_rps, demand.yaw_rps), (0.4, -0.2));
    assert!(demand.payload_write.is_some());
    assert!(
        control
            .snapshot_cached()
            .unwrap()
            .progress
            .sent_at
            .is_none()
    );
}
#[test]
fn another_owner_or_another_device_never_grants_authority() {
    let (mut control, mut commands, rates) = control();
    observe(&mut control, status(1, 2, (42, 190)));
    observe(
        &mut control,
        status(2, 3, (GCS_SYSTEM_ID, GCS_COMPONENT_ID)),
    );
    assert!(!control.rate_demand(1.0, 0.0));
    assert_eq!(commands.try_recv().unwrap().command, 512);
    assert!(commands.try_recv().is_err());
    assert!(rates.borrow().is_none());
}
#[test]
fn manager_ownership_loss_fences_queued_motion() {
    let (mut control, _commands, rates) = control();
    owned(&mut control);
    assert!(control.rate_demand(1.0, 0.0));
    let demand = rates.borrow().clone().unwrap();
    observe(&mut control, status(2, 2, (42, 190)));
    assert!(!demand.fence.unwrap().is_valid());
    assert_eq!(
        control.snapshot_cached().unwrap().progress.ended,
        Some(turret_core::EndReason::Revoked)
    );
}
#[test]
fn recenter_is_not_a_continuous_action_and_idle_does_not_replace_it() {
    let (mut control, mut commands, rates) = control();
    owned(&mut control);
    control.bind_timing(None);
    assert!(control.neutral());
    assert_eq!(commands.try_recv().unwrap().command, 512);
    let command = commands.try_recv().unwrap();
    assert_eq!(command.command, 1000);
    let snapshot = control.snapshot_cached().unwrap();
    assert!(snapshot.command.timing.valid_until.is_none());
    assert!(control.rate_demand(0.0, 0.0));
    assert!(rates.borrow().is_none());
    assert_eq!(
        control.snapshot_cached().unwrap().command.identity,
        snapshot.command.identity
    );
}
#[test]
fn link_loss_fences_previous_generation_and_queues_stop_without_stealing_authority() {
    let (mut control, mut commands, rates) = control();
    owned(&mut control);
    assert!(control.rate_demand(1.0, 0.0));
    let prior = rates.borrow().clone().unwrap();
    assert_eq!(commands.try_recv().unwrap().command, 512);
    assert!(control.queue_link_loss_stop());
    assert!(!prior.fence.unwrap().is_valid());
    let stop = rates.borrow().clone().unwrap();
    assert_eq!((stop.pitch_rps, stop.yaw_rps), (0.0, 0.0));
    assert!(commands.try_recv().is_err());
}

#[test]
fn rejected_replacement_does_not_cancel_pending_stop_responsibility() {
    let (mut control, _commands, rates) = control();
    owned(&mut control);
    assert!(control.rate_demand(1.0, 0.0));
    assert!(control.queue_link_loss_stop());
    let stop = rates.borrow().clone().unwrap();
    control.timing = Some(PayloadTiming {
        send_before: Instant::now(),
        valid_until: None,
        ack_before: None,
        completion_before: None,
    });
    assert!(!control.rate_demand(f32::NAN, 0.0));
    assert!(stop.fence.unwrap().is_valid());
    assert!(control.release_after_stop.is_some());
    assert_eq!(rates.borrow().as_ref().unwrap().pitch_rps, 0.0);
}
