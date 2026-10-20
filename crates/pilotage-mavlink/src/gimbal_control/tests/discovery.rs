use super::*;

fn raw(control: &ManagerControl, component: u8, id: u32, crc: u8, payload: &[u8]) {
    let mut bytes = [0; 280];
    let length = crate::codec::encode_frame_v2_from(1, id, payload, crc, 1, component, &mut bytes);
    let mut messages = Vec::new();
    let stats = crate::parse_datagram(&bytes[..length], &mut messages);
    assert_eq!(stats.crc_failures, 0);
    assert_eq!(messages.len(), 1);
    crate::link::apply_messages_at(
        control.manager_state.as_ref().unwrap(),
        &messages,
        0,
        0,
        Instant::now(),
    );
}

fn information(control: &ManagerControl, component: u8, device: u8, time: u32, flags: u32) {
    raw(
        control,
        component,
        280,
        70,
        &information_payload(device, time, flags),
    );
}

fn information_payload(device: u8, time: u32, flags: u32) -> [u8; 33] {
    let mut payload = [0; 33];
    payload[..4].copy_from_slice(&time.to_le_bytes());
    payload[4..8].copy_from_slice(&flags.to_le_bytes());
    for (index, limit) in [f32::NAN, f32::NAN, -1.5, 1.5, -3.0, 3.0]
        .into_iter()
        .enumerate()
    {
        payload[8 + index * 4..12 + index * 4].copy_from_slice(&limit.to_le_bytes());
    }
    payload[32] = device;
    payload
}

fn ownership(control: &ManagerControl, component: u8, device: u8, time: u32) {
    let mut payload = [0; 13];
    payload[..4].copy_from_slice(&time.to_le_bytes());
    payload[8] = device;
    payload[9] = GCS_SYSTEM_ID;
    payload[10] = GCS_COMPONENT_ID;
    raw(control, component, 281, 48, &payload);
}

const FLAGS: u32 = 2 | 32 | 64 | 128 | 256 | 512;

#[test]
fn wrapped_manager_requests_information_and_recovers_only_after_rediscovery() {
    let (mut control, mut commands, rates) = control();
    information(&control, 1, 2, u32::MAX - 2, FLAGS);
    ownership(&control, 1, 2, u32::MAX - 1);
    assert!(control.rate_demand(0.5, 0.0));
    let queued = rates.borrow().clone().unwrap();
    while commands.try_recv().is_ok() {}
    ownership(&control, 1, 2, 1);
    control.next_probe = Instant::now();
    control.maintain();
    assert!(!queued.fence.unwrap().with_valid_write(|| Ok(true)).unwrap());
    assert!(!control.rate_demand(0.5, 0.0));
    let mut requested_information = false;
    while let Ok(command) = commands.try_recv() {
        requested_information |= command.command == 512 && command.params[0] == 280.0;
    }
    assert!(requested_information);
    information(&control, 1, 2, 2, FLAGS);
    ownership(&control, 1, 2, 3);
    assert!(control.rate_demand(0.5, 0.0));
    let demand = rates.borrow().clone().unwrap();
    assert_eq!((demand.target_component, demand.gimbal_device_id), (1, 2));
    assert!(demand.fence.unwrap().with_valid_write(|| Ok(true)).unwrap());
}

#[test]
fn multiple_discovered_managers_require_explicit_selection() {
    let (commands, mut receiver) = mpsc::channel(16);
    let (rates, observer) = watch::channel(None);
    let mut control = ManagerControl::new(commands, rates, 1, None)
        .unwrap()
        .with_manager_state(std::sync::Arc::new(std::sync::Mutex::new(
            crate::LinkState::default(),
        )));
    for (component, device) in [(191, 154), (192, 155)] {
        information(&control, component, device, 1, FLAGS);
        ownership(&control, component, device, 2);
    }
    assert!(!control.rate_demand(0.5, 0.0));
    assert!(control.selected_device().is_none());
    assert!(observer.borrow().is_none());
    while let Ok(command) = receiver.try_recv() {
        assert_eq!(command.command, 512);
    }
}

#[test]
fn independent_manager_target_and_contract_revision_are_bound_at_final_write() {
    let (commands, _receiver) = mpsc::channel(16);
    let (rates, observer) = watch::channel(None);
    let mut control = ManagerControl::new(commands, rates, 1, Some((191, 154)))
        .unwrap()
        .with_manager_state(std::sync::Arc::new(std::sync::Mutex::new(
            crate::LinkState::default(),
        )));
    information(&control, 191, 154, 1, FLAGS);
    ownership(&control, 191, 154, 2);
    assert!(control.rate_demand(0.5, 0.0));
    let demand = observer.borrow().clone().unwrap();
    assert_eq!((demand.target_system, demand.target_component), (1, 191));
    assert_eq!(demand.gimbal_device_id, 154);
    let fence = demand.fence.unwrap();
    assert!(fence.with_valid_write(|| Ok(true)).unwrap());
    information(&control, 191, 154, 3, FLAGS | 4);
    assert!(!fence.with_valid_write(|| Ok(true)).unwrap());
}

#[test]
fn invalid_limits_revoke_the_known_contract_until_valid_discovery_arrives() {
    let (mut control, _commands, rates) = control();
    information(&control, 1, 2, 1, FLAGS);
    ownership(&control, 1, 2, 2);
    assert!(control.rate_demand(0.5, 0.0));
    let queued = rates.borrow().clone().unwrap();
    let mut malformed = information_payload(2, 3, FLAGS);
    malformed[20..24].copy_from_slice(&f32::INFINITY.to_le_bytes());
    raw(&control, 1, 280, 70, &malformed);
    assert!(!queued.fence.unwrap().with_valid_write(|| Ok(true)).unwrap());
    control.refresh_discovery();
    assert!(matches!(
        control.capabilities().gimbal.axes.pitch.rate,
        turret_core::Capability::Unknown
    ));
    assert!(matches!(
        control.capabilities().gimbal.recenter,
        turret_core::Capability::Unknown
    ));
    assert!(!control.rate_demand(0.5, 0.0));
    assert!(!control.command(PayloadCommand::Recenter));
    assert!(rates.borrow().is_none());
    information(&control, 1, 2, 4, FLAGS);
    ownership(&control, 1, 2, 5);
    assert!(control.rate_demand(0.5, 0.0));
    assert!(
        rates
            .borrow()
            .as_ref()
            .unwrap()
            .fence
            .as_ref()
            .unwrap()
            .with_valid_write(|| Ok(true))
            .unwrap()
    );
}

#[test]
fn typed_commands_preserve_each_supported_frame_and_omit_roll_in_the_transport_demand() {
    use turret_core::{Axes, GimbalRate, ReferenceFrame};

    let (mut control, _commands, rates) = control();
    information(&control, 1, 2, 1, FLAGS | 1024);
    ownership(&control, 1, 2, 2);
    for (frame, expected_flags) in [
        (ReferenceFrame::Vehicle, 0),
        (ReferenceFrame::HorizonVehicleYaw, 4 | 8),
        (ReferenceFrame::EarthNed, 4 | 8 | 16),
    ] {
        let payload = PayloadCommand::GimbalRate(GimbalRate {
            rates: Axes {
                pitch: Some(AngularRate::radians_per_second(0.2).unwrap()),
                roll: None,
                yaw: Some(AngularRate::radians_per_second(-0.3).unwrap()),
            },
            frame,
        });
        assert!(control.command(payload));
        let demand = rates.borrow().clone().unwrap();
        assert_eq!(demand.flags, expected_flags);
        assert_eq!((demand.pitch_rps, demand.yaw_rps), (0.2, -0.3));
        assert!(demand.roll_rps.is_nan());
        assert_eq!((demand.target_component, demand.gimbal_device_id), (1, 2));
        assert!(demand.fence.unwrap().with_valid_write(|| Ok(true)).unwrap());
    }
}
