use crate::tests::{admit, engine, grant_scope};
use crate::{ClientAction, ModuleEvent, TransportEvent};
use pilotage_protocol::wire;

fn catalog(revision: u64) -> wire::HostCapabilities {
    wire::HostCapabilities {
        vehicles: vec![wire::VehicleDescriptor {
            vehicle: Some(wire::VehicleId { value: 1 }),
            scopes: vec![
                wire::ScopeDescriptor {
                    scope: Some(wire::ScopeId {
                        value: "vehicle.motion".into(),
                    }),
                    ..Default::default()
                },
                wire::ScopeDescriptor {
                    scope: Some(wire::ScopeId {
                        value: "vehicle.camera.100.0".into(),
                    }),
                    payload: Some(wire::PayloadCapabilities {
                        endpoint_id: "camera".into(),
                        revision,
                        connection_incarnation: vec![5; 16],
                        connection_epoch: 3,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn refresh(engine: &mut crate::ClientEngine, revision: u64) -> Vec<ClientAction> {
    let envelope = wire::Envelope {
        schema_version: 1,
        payload: Some(wire::envelope::Payload::HostCapabilities(catalog(revision))),
    };
    engine.handle(
        TransportEvent::BootstrapReceived(pilotage_protocol::encode_envelope_length_delimited(
            &envelope,
        )),
        0,
    )
}

#[test]
fn live_catalog_authorizes_identity_and_changes_only_affected_lanes() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    let result = refresh(&mut engine, 1);
    assert!(result.iter().any(|action| matches!(
        action,
        ClientAction::Emit(ModuleEvent::CapabilitiesChanged(_))
    )));
    engine.request_lease(1, "vehicle.motion");
    grant_scope(&mut engine, "vehicle.motion", 4);
    engine.request_lease(1, "vehicle.camera.100.0");
    grant_scope(&mut engine, "vehicle.camera.100.0", 2);
    assert!(engine.holds(1, "vehicle.camera.100.0"));
    refresh(&mut engine, 2);
    assert!(engine.holds(1, "vehicle.motion"));
    assert!(!engine.holds(1, "vehicle.camera.100.0"));
    let payload = engine.admission().unwrap().vehicles[0].scopes[1]
        .payload
        .as_ref()
        .unwrap();
    assert_eq!(payload.connection_incarnation, vec![5; 16]);
    assert_eq!(payload.connection_epoch, 3);
}

#[test]
fn old_pending_lease_and_transfer_cannot_reopen_replaced_contract() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    refresh(&mut engine, 1);
    engine.request_lease(1, "vehicle.camera.100.0");
    engine.request_takeover(1, "vehicle.camera.100.0");
    refresh(&mut engine, 2);
    grant_scope(&mut engine, "vehicle.camera.100.0", 2);
    assert!(!engine.holds(1, "vehicle.camera.100.0"));
    assert!(!engine.takeover_pending(1, "vehicle.camera.100.0"));
    engine.request_lease(1, "vehicle.camera.100.0");
    grant_scope(&mut engine, "vehicle.camera.100.0", 4);
    assert!(engine.holds(1, "vehicle.camera.100.0"));
}

#[test]
fn a_command_only_contract_change_closes_the_held_lane() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    assert!(!engine.admission().unwrap().vehicles[0].scopes[0].command_only);
    engine.request_lease(1, "vehicle.motion");
    grant_scope(&mut engine, "vehicle.motion", 4);
    assert!(engine.holds(1, "vehicle.motion"));
    let mut changed = catalog(1);
    changed.vehicles[0].scopes[0].command_only = true;
    let envelope = wire::Envelope {
        schema_version: pilotage_protocol::SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::HostCapabilities(changed)),
    };
    engine.handle(
        TransportEvent::BootstrapReceived(pilotage_protocol::encode_envelope_length_delimited(
            &envelope,
        )),
        1,
    );
    assert!(engine.admission().unwrap().vehicles[0].scopes[0].command_only);
    assert!(!engine.holds(1, "vehicle.motion"));
    engine.request_lease(1, "vehicle.motion");
    grant_scope(&mut engine, "vehicle.motion", 6);
    assert!(engine.holds(1, "vehicle.motion"));
}
