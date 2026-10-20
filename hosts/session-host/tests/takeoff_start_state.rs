//! Start-state requirements pass from adapter capabilities through admission.

#![cfg(test)]

use std::time::Duration;

use pilotage_adapter_api::{
    ActionCapability, AdapterCapabilities, ExecutionMode, ScopeDescriptor, VehicleDescriptor,
};
use pilotage_client_session::{ClientConfig, ClientEngine, TransportEvent};
use pilotage_protocol::{
    ActionKind, ClientHello, ScopeId, TakeoffStartState, VehicleClass, VehicleId,
    encode_envelope_length_delimited, wire,
};
use pilotage_session::{
    ClientKey, DomainEnvelope, OutboundMessage, SessionAction, SessionConfig, SessionEngine,
};
use pilotage_timing::{MonoTimestamp, StalenessPolicy};

fn capabilities(state: Option<TakeoffStartState>) -> AdapterCapabilities {
    AdapterCapabilities {
        execution: ExecutionMode::default(),
        vehicles: vec![VehicleDescriptor {
            id: VehicleId::new(7),
            class: VehicleClass::Multicopter,
            heading_reference: None,
            display_name: "test vehicle".to_owned(),
            scopes: vec![ScopeDescriptor {
                payload: None,
                authority_group: None,
                scope: ScopeId::new("vehicle.motion"),
                axes: vec![],
                intents: vec![],
                actions: vec![
                    ActionCapability {
                        action: ActionKind::Arm,
                        mode_targets: vec![],
                        feel_targets: vec![],
                        takeoff_start_state: None,
                    },
                    ActionCapability {
                        action: ActionKind::Takeoff,
                        mode_targets: vec![],
                        feel_targets: vec![],
                        takeoff_start_state: state,
                    },
                ],
                legacy: None,
            }],
            link_loss_actions: vec![],
        }],
        adapter_version: "test".to_owned(),
        control_feel: None,
    }
}

fn welcome(state: Option<TakeoffStartState>) -> wire::ServerWelcome {
    let mut host = SessionEngine::new(
        capabilities(state),
        StalenessPolicy::new(Duration::from_millis(50)),
        SessionConfig::new(1, "test"),
    );
    let outcome = host.handle_client_message(
        ClientKey::new(1),
        DomainEnvelope::Hello(ClientHello {
            protocol_version: 1,
            client_name: "capability-reader".to_owned(),
            join_token: vec![],
        }),
        MonoTimestamp::from_nanos(0),
    );
    let [
        SessionAction::SendToClient {
            envelope: OutboundMessage::Welcome(welcome),
            ..
        },
    ] = outcome.actions.as_slice()
    else {
        panic!("expected welcome, got {:?}", outcome.actions);
    };
    wire::ServerWelcome::from(welcome)
}

fn client_actions(welcome: wire::ServerWelcome) -> Vec<wire::ActionCapability> {
    let envelope = wire::Envelope {
        schema_version: pilotage_protocol::SCHEMA_VERSION,
        payload: Some(wire::envelope::Payload::ServerWelcome(welcome)),
    };
    let mut client = ClientEngine::new(ClientConfig {
        client_name: "capability-reader".to_owned(),
        reconnect: Default::default(),
        join_token: None,
    });
    client.handle(TransportEvent::Connected, 0);
    client.handle(
        TransportEvent::BootstrapReceived(encode_envelope_length_delimited(&envelope)),
        1,
    );
    let admission = client.admission().unwrap();
    assert_eq!(admission.vehicles[0].vehicle_id, 7);
    assert_eq!(admission.vehicles[0].scopes[0].scope, "vehicle.motion");
    admission.vehicles[0].scopes[0].actions.clone()
}

#[test]
fn the_published_start_requirement_reaches_the_client_catalog() {
    for state in [
        None,
        Some(TakeoffStartState::ArmedOnGround),
        Some(TakeoffStartState::MotorsStoppedOnGround),
    ] {
        let actions = client_actions(welcome(state));
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].action, wire::ControlAction::Arm as i32);
        assert_eq!(
            TakeoffStartState::from_wire(actions[0].takeoff_start_state),
            None
        );
        assert_eq!(actions[1].action, wire::ControlAction::Takeoff as i32);
        assert_eq!(
            TakeoffStartState::from_wire(actions[1].takeoff_start_state),
            state
        );
    }
}

#[test]
fn an_unrecognized_start_requirement_remains_unknown_in_the_catalog() {
    let mut welcome = welcome(None);
    welcome.host_capabilities.as_mut().unwrap().vehicles[0].scopes[0].actions[1]
        .takeoff_start_state = 42;
    let actions = client_actions(welcome);
    assert_eq!(actions[1].takeoff_start_state, 42);
    assert_eq!(
        TakeoffStartState::from_wire(actions[1].takeoff_start_state),
        None
    );
}
