use super::*;

#[test]
fn undiscovered_gimbal_exposes_no_rate_axes_or_recenter_action() {
    let (control, _lanes) = gimbal_control();
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), live_state());
    adapter.gimbal = Some(control);
    let capabilities = adapter.capabilities();
    let scope = capabilities.vehicles[0]
        .scopes
        .iter()
        .find(|scope| scope.scope.as_str() == super::super::GIMBAL_SCOPE)
        .unwrap();
    assert!(scope.axes.is_empty());
    assert!(scope.intents.is_empty());
    assert!(scope.actions.is_empty());
}

#[test]
fn pitch_only_discovery_removes_yaw_and_recenter_from_the_app_contract() {
    let state = live_state();
    let (control, _lanes) = gimbal_control();
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), state.clone()).with_gimbal(control);
    apply_messages_at(
        &state,
        &[(
            SOURCE,
            FcMessage::GimbalManagerInformation(
                pilotage_mavlink::codec::GimbalManagerInformation {
                    time_boot_ms: 3,
                    cap_flags: 32 | 128,
                    roll_min: f32::NAN,
                    roll_max: f32::NAN,
                    pitch_min: -1.5,
                    pitch_max: 1.5,
                    yaw_min: f32::NAN,
                    yaw_max: f32::NAN,
                    device_id: 2,
                },
            ),
        )],
        0,
        0,
        Instant::now(),
    );
    adapter.sample_telemetry();
    let capabilities = adapter.capabilities();
    let scope = capabilities.vehicles[0]
        .scopes
        .iter()
        .find(|scope| scope.scope.as_str() == super::super::GIMBAL_SCOPE)
        .unwrap();
    assert_eq!(
        scope.axes,
        vec![LogicalAxisId::new(super::super::PITCH_AXIS)]
    );
    assert_eq!(scope.intents.len(), 1);
    assert!(scope.actions.is_empty());
    assert!(matches!(
        scope.legacy,
        Some(pilotage_adapter_api::LegacyCommandMap::GimbalRate {
            pitch: Some(_),
            yaw: None,
            recenter_button: None,
        })
    ));
}

#[test]
fn payload_catalog_and_command_use_one_connection_identity_without_feedback() {
    let (control, _lanes) = gimbal_control();
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), live_state()).with_gimbal(control);
    let outcome = adapter.apply_control(&gimbal_frame(
        vec![(LogicalAxisId::new(super::super::PITCH_AXIS), 0.5)],
        vec![],
    ));
    assert_eq!(outcome.disposition, Disposition::Accepted);
    let capabilities = adapter.capabilities();
    let contract = capabilities.vehicles[0]
        .scopes
        .iter()
        .find(|scope| scope.scope.as_str() == super::super::GIMBAL_SCOPE)
        .unwrap()
        .payload
        .as_ref()
        .unwrap();
    let state = adapter
        .sample_telemetry()
        .samples
        .into_iter()
        .find_map(|sample| sample.payload)
        .unwrap();
    assert_eq!(
        state.connection_incarnation,
        contract.connection_incarnation
    );
    assert_eq!(state.connection_epoch, contract.connection_epoch);
    assert_eq!(
        state.command.unwrap().connection_epoch,
        contract.connection_epoch
    );
    assert!(state.pitch.is_none());
}
