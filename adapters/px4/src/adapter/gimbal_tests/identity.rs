use super::*;

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
