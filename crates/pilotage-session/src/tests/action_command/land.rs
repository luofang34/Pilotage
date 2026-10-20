use super::*;
use pilotage_adapter_api::ActionCapability;
use pilotage_protocol::{ActionKind, CommandTiming};

fn land_engine() -> (SessionEngine, ClientKey, SessionId, Generation) {
    let mut capabilities = super::super::capabilities();
    let scope = &mut capabilities.vehicles[0].scopes[0];
    scope.axes.clear();
    scope.intents.clear();
    scope.legacy = None;
    scope.actions = vec![ActionCapability {
        action: ActionKind::Land,
        mode_targets: vec![],
        feel_targets: vec![],
        takeoff_start_state: None,
    }];
    let mut engine = SessionEngine::new(
        capabilities,
        super::super::staleness(),
        crate::SessionConfig::new(1, "host-test"),
    );
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    announce(&mut engine, client, session, 1);
    let generation = grant_motion(&mut engine, client);
    (engine, client, session, generation)
}

#[test]
fn a_command_only_scope_delivers_land_with_the_original_identity() {
    let (mut engine, client, session, generation) = land_engine();
    let outcome = engine.handle_client_message(
        client,
        command(session, generation, ControlAction::Land, 31),
        MonoTimestamp::from_nanos(10),
    );
    match outcome.actions.as_slice() {
        [SessionAction::ApplyToAdapter { frame, .. }] => {
            assert_eq!(frame.actions, vec![ControlAction::Land]);
            assert_eq!(frame.action_ids, vec![31]);
            assert_eq!(frame.session, session);
            assert_eq!(frame.generation, generation);
            assert_eq!(frame.scope, motion());
            assert_eq!(frame.activation_revision, 1);
            assert!(frame.intent.is_none() && !frame.carries_payload());
        }
        other => panic!("expected one landing request, got {other:?}"),
    }
}

#[test]
fn disarm_and_takeoff_support_do_not_admit_land() {
    let (mut engine, client, session, generation) = super::takeoff::takeoff_engine();
    let outcome = engine.handle_client_message(
        client,
        command(session, generation, ControlAction::Land, 32),
        MonoTimestamp::from_nanos(10),
    );
    assert!(rejected_detail(&outcome).contains("not advertised"));
}

#[test]
fn a_land_only_scope_does_not_admit_disarm_or_takeoff() {
    let (mut engine, client, session, generation) = land_engine();
    for action in [ControlAction::Disarm, ControlAction::Takeoff] {
        let outcome = engine.handle_client_message(
            client,
            command(session, generation, action, 33),
            MonoTimestamp::from_nanos(10),
        );
        assert!(rejected_detail(&outcome).contains("not advertised"));
    }
}

#[test]
fn land_from_a_released_generation_never_reaches_the_adapter() {
    let (mut engine, client, session, generation) = land_engine();
    let released = engine.handle_client_message(
        client,
        DomainEnvelope::Release(LeaseRelease {
            vehicle: VEHICLE,
            scope: motion(),
        }),
        MonoTimestamp::from_nanos(10),
    );
    assert!(!released.actions.is_empty());
    assert_ne!(grant_motion(&mut engine, client), generation);
    let outcome = engine.handle_client_message(
        client,
        command(session, generation, ControlAction::Land, 34),
        MonoTimestamp::from_nanos(12),
    );
    assert!(rejected_detail(&outcome).contains("stale generation"));
}

#[test]
fn a_queued_land_request_expires_before_adapter_delivery() {
    let (mut engine, client, session, generation) = land_engine();
    let mut message = command(session, generation, ControlAction::Land, 35);
    if let DomainEnvelope::ActionCommand(value) = &mut message {
        value.timing = Some(CommandTiming::default());
    }
    let outcome = engine.handle_received_message(
        client,
        message,
        MonoTimestamp::from_nanos(10_000_000),
        MonoTimestamp::from_nanos(260_000_000),
    );
    assert!(rejected_detail(&outcome).contains("timing expired"));
}
