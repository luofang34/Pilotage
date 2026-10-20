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

#[test]
fn startup_capability_changes_fence_unused_scopes_without_engaging_link_loss() {
    let mut engine = engine();
    let mut capabilities = capabilities();
    for (limit, expected_generation) in [(2.0, 1), (3.0, 2)] {
        capabilities.vehicles[0].scopes[0].intents[0].max_linear = limit;
        let outcome = engine.refresh_capabilities(
            capabilities.clone(),
            MonoTimestamp::from_nanos(expected_generation),
        );
        assert_eq!(engaged_neutralize(&outcome), 0);
        assert!(outcome.actions.iter().any(|action| matches!(
            action,
            SessionAction::InvalidatePendingCommands { vehicle, scope }
                if *vehicle == VEHICLE && *scope == motion()
        )));
        assert!(outcome.actions.iter().any(|action| matches!(
            action,
            SessionAction::Broadcast {
                envelope: OutboundMessage::Authority(AuthorityEffect::ScopeLeaseRevoked {
                    previous_holder: None, generation, ..
                })
            } if generation.as_u64() == expected_generation
        )));
        assert!(outcome.actions.iter().any(|action| matches!(
            action,
            SessionAction::Broadcast {
                envelope: OutboundMessage::Capabilities(_)
            }
        )));
    }
    assert!(
        engine
            .handle_tick(MonoTimestamp::from_nanos(3))
            .actions
            .is_empty()
    );
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    let generation = grant(&mut engine, client, 4);
    let neutral = engine.handle_client_message(
        client,
        DomainEnvelope::Frame(neutral_frame(
            session,
            generation,
            SequenceNum::new(1),
            MonoTimestamp::from_nanos(5),
        )),
        MonoTimestamp::from_nanos(5),
    );
    assert_eq!(cleared(&neutral), 0);
    assert!(
        neutral
            .actions
            .iter()
            .any(|action| matches!(action, SessionAction::ApplyToAdapter { .. }))
    );
}

#[test]
fn capability_changes_still_engage_link_loss_when_a_real_holder_is_revoked() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    let generation = grant(&mut engine, client, 1);
    let mut capabilities = capabilities();
    capabilities.vehicles[0].scopes[0].intents[0].max_linear = 2.0;
    let outcome = engine.refresh_capabilities(capabilities, MonoTimestamp::from_nanos(2));
    assert_eq!(engaged_neutralize(&outcome), 1);
    assert_eq!(
        engage_trigger(&outcome),
        Some(LinkLossTrigger::AuthorityRevoked)
    );
    assert!(outcome.actions.iter().any(|action| matches!(
        action,
        SessionAction::Broadcast {
            envelope: OutboundMessage::Authority(AuthorityEffect::ScopeLeaseRevoked {
                previous_holder: Some(_), generation: revoked, ..
            })
        } if revoked.as_u64() == generation.as_u64().wrapping_add(1)
    )));
    let stale = engine.handle_client_message(
        client,
        DomainEnvelope::Frame(frame(
            session,
            generation,
            SequenceNum::new(1),
            MonoTimestamp::from_nanos(3),
        )),
        MonoTimestamp::from_nanos(3),
    );
    assert!(
        !stale
            .actions
            .iter()
            .any(|action| matches!(action, SessionAction::ApplyToAdapter { .. }))
    );
    let renewed = grant(&mut engine, client, 4);
    let neutral = engine.handle_client_message(
        client,
        DomainEnvelope::Frame(neutral_frame(
            session,
            renewed,
            SequenceNum::new(2),
            MonoTimestamp::from_nanos(5),
        )),
        MonoTimestamp::from_nanos(5),
    );
    assert_eq!(cleared(&neutral), 1);
}
