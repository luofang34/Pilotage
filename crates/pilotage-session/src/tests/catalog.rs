use super::*;

fn catalog(revision: u64) -> AdapterCapabilities {
    let mut result = capabilities();
    let mut camera = result.vehicles[0].scopes[0].clone();
    camera.scope = ScopeId::new("vehicle.camera.100.0");
    camera.axes.clear();
    camera.intents.clear();
    camera.legacy = None;
    camera.payload = Some(pilotage_protocol::wire::PayloadCapabilities {
        revision,
        endpoint_id: "camera".to_owned(),
        connection_incarnation: vec![8; 16],
        connection_epoch: 1,
        ..Default::default()
    });
    result.vehicles[0].scopes.push(camera);
    result
}

fn lease(engine: &mut SessionEngine, client: ClientKey, scope: &str) -> Generation {
    let result = engine.handle_client_message(
        client,
        DomainEnvelope::Lease(LeaseRequest {
            vehicle: VEHICLE,
            scope: ScopeId::new(scope),
        }),
        MonoTimestamp::from_nanos(10),
    );
    result
        .actions
        .iter()
        .find_map(|action| match action {
            SessionAction::SendToClient {
                envelope: OutboundMessage::LeaseResponse(reply),
                ..
            } if reply.granted => Some(reply.generation),
            _ => None,
        })
        .unwrap()
}

#[test]
fn discovery_registers_camera_and_contract_changes_revoke_only_its_lease() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    welcome(&mut engine, client);
    let motion_generation = lease(&mut engine, client, "vehicle.motion");
    let added = engine.refresh_capabilities(catalog(1), MonoTimestamp::from_nanos(11));
    assert!(added.actions.iter().any(|action| matches!(
        action,
        SessionAction::Broadcast {
            envelope: OutboundMessage::Capabilities(_)
        }
    )));
    assert!(
        !added
            .actions
            .iter()
            .any(|action| matches!(action, SessionAction::InvalidatePendingCommands { .. }))
    );
    let camera_generation = lease(&mut engine, client, "vehicle.camera.100.0");
    let changed = engine.refresh_capabilities(catalog(2), MonoTimestamp::from_nanos(12));
    assert!(changed.actions.iter().any(|action| matches!(action,
        SessionAction::InvalidatePendingCommands { scope, .. } if scope.as_str() == "vehicle.camera.100.0")));
    assert!(
        !changed
            .actions
            .iter()
            .any(|action| matches!(action, SessionAction::EngageLinkLoss { .. }))
    );
    assert_eq!(
        lease(&mut engine, client, "vehicle.motion"),
        motion_generation
    );
    assert_ne!(
        lease(&mut engine, client, "vehicle.camera.100.0"),
        camera_generation
    );
    assert!(
        engine
            .refresh_capabilities(catalog(2), MonoTimestamp::from_nanos(13))
            .actions
            .is_empty()
    );
}
