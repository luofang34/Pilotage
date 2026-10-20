use super::*;
use pilotage_adapter_api::ActionCapability;
use pilotage_protocol::{ActionKind, ModeTarget};

fn takeoff_engine() -> (SessionEngine, ClientKey, SessionId, Generation) {
    let mut capabilities = super::super::capabilities();
    capabilities.vehicles[0].class = pilotage_protocol::VehicleClass::Multicopter;
    capabilities.vehicles[0].scopes[0]
        .actions
        .push(ActionCapability {
            action: ActionKind::Takeoff,
            mode_targets: vec![],
            feel_targets: vec![],
            takeoff_start_state: Some(pilotage_protocol::TakeoffStartState::ArmedOnGround),
        });
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
fn arm_and_mode_support_do_not_admit_takeoff() {
    let mut capabilities = super::super::capabilities();
    capabilities.vehicles[0].scopes[0]
        .actions
        .push(ActionCapability {
            action: ActionKind::ModeRequest,
            mode_targets: vec![ModeTarget::CameraVelocity],
            feel_targets: vec![],
            takeoff_start_state: None,
        });
    let mut engine = SessionEngine::new(
        capabilities,
        super::super::staleness(),
        crate::SessionConfig::new(1, "host-test"),
    );
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    announce(&mut engine, client, session, 1);
    let generation = grant_motion(&mut engine, client);
    let outcome = engine.handle_client_message(
        client,
        command(session, generation, ControlAction::Takeoff, 20),
        MonoTimestamp::from_nanos(10),
    );
    assert!(rejected_detail(&outcome).contains("not advertised"));
}

#[test]
fn advertised_takeoff_keeps_its_meaning_and_command_identity() {
    let (mut engine, client, session, generation) = takeoff_engine();
    let outcome = engine.handle_client_message(
        client,
        command(session, generation, ControlAction::Takeoff, 21),
        MonoTimestamp::from_nanos(10),
    );
    match outcome.actions.as_slice() {
        [SessionAction::ApplyToAdapter { frame, .. }] => {
            assert_eq!(frame.actions, vec![ControlAction::Takeoff]);
            assert_eq!(frame.action_ids, vec![21]);
            assert_eq!(frame.generation, generation);
            assert!(frame.intent.is_none());
        }
        other => panic!("expected takeoff delivery, got {other:?}"),
    }
}

#[test]
fn takeoff_after_authority_release_never_reaches_the_adapter() {
    let (mut engine, client, session, generation) = takeoff_engine();
    let released = engine.handle_client_message(
        client,
        DomainEnvelope::Release(LeaseRelease {
            vehicle: VEHICLE,
            scope: motion(),
        }),
        MonoTimestamp::from_nanos(10),
    );
    assert!(!released.actions.is_empty());
    let fresh = grant_motion(&mut engine, client);
    assert_ne!(fresh, generation);
    let outcome = engine.handle_client_message(
        client,
        command(session, generation, ControlAction::Takeoff, 22),
        MonoTimestamp::from_nanos(12),
    );
    assert!(rejected_detail(&outcome).contains("stale generation"));
}

#[test]
fn queued_takeoff_expires_before_adapter_delivery() {
    let (mut engine, client, session, generation) = takeoff_engine();
    let mut message = command(session, generation, ControlAction::Takeoff, 23);
    if let DomainEnvelope::ActionCommand(value) = &mut message {
        value.timing = Some(pilotage_protocol::CommandTiming::default());
    }
    let outcome = engine.handle_received_message(
        client,
        message,
        MonoTimestamp::from_nanos(10_000_000),
        MonoTimestamp::from_nanos(260_000_000),
    );
    assert!(rejected_detail(&outcome).contains("timing expired"));
}
