//! Scope routing under multiple held lanes: flight demands and the
//! arm/disarm pair belong to the motion lane by NAME. A driver that
//! infers "the" lane from whichever sorts first routes flight into the
//! gimbal's fencing the moment that lease lands — the motion frames
//! vanish and the host's silence watchdog revokes flight one second
//! later, with the gimbal stream flowing the whole time.

#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Instant;

use pilotage_client_session::{
    ClientAction, ClientConfig, ClientEngine, MotionDemand, ReconnectPolicy, TransportEvent,
};
use pilotage_control_web::ControlCoordinator;
use pilotage_protocol::wire;
use prost::Message;

use super::delivery::DeliveryQueue;
use super::driver::{Link, LinkStats};
use super::observer::LinkObserver;
use super::records::LinkEvent;

struct SilentObserver;

impl LinkObserver for SilentObserver {
    fn on_event(&self, _event: LinkEvent) {}
    fn on_state_frame(&self, _frame: Vec<u8>, _accepted_at_ms: u64) {}
    fn on_video_frame(&self, _source_id: u8, _codec: String, _payload: Vec<u8>) {}
}

/// A welcome offering one vehicle with a velocity motion scope and a
/// gimbal-rate scope.
fn two_scope_welcome() -> wire::Envelope {
    wire::Envelope {
        schema_version: 1,
        payload: Some(wire::envelope::Payload::ServerWelcome(
            wire::ServerWelcome {
                session: Some(wire::SessionId { value: 7 }),
                principal: Some(wire::PrincipalId { value: 42 }),
                host_capabilities: Some(wire::HostCapabilities {
                    host_version: "test-host".into(),
                    vehicles: vec![wire::VehicleDescriptor {
                        vehicle: Some(wire::VehicleId { value: 1 }),
                        display_name: "vehicle".into(),
                        scopes: vec![
                            wire::ScopeDescriptor {
                                scope: Some(wire::ScopeId {
                                    value: "vehicle.motion".into(),
                                }),
                                intents: vec![wire::IntentCapability {
                                    family: wire::IntentFamily::Velocity as i32,
                                    frames: vec![wire::ReferenceFrame::BodyFrd as i32],
                                    max_linear: 3.0,
                                    max_vertical: 1.5,
                                    max_angular: 0.9,
                                    ..Default::default()
                                }],
                                ..Default::default()
                            },
                            wire::ScopeDescriptor {
                                scope: Some(wire::ScopeId {
                                    value: "vehicle.gimbal".into(),
                                }),
                                intents: vec![wire::IntentCapability {
                                    family: wire::IntentFamily::GimbalRate as i32,
                                    max_angular: 0.8,
                                    ..Default::default()
                                }],
                                ..Default::default()
                            },
                        ],
                        supported_modes: Vec::new(),
                    }],
                    supported_modes: Vec::new(),
                }),
                ..Default::default()
            },
        )),
    }
}

fn admitted_link_with_two_lanes() -> Link {
    let mut engine = ClientEngine::new(ClientConfig {
        client_name: "routing-test".into(),
        reconnect: ReconnectPolicy::default(),
    });
    engine.handle(TransportEvent::Connected, 0);
    engine.handle(
        TransportEvent::BootstrapReceived(pilotage_protocol::encode_envelope_length_delimited(
            &two_scope_welcome(),
        )),
        0,
    );
    for (scope, generation) in [("vehicle.motion", 4), ("vehicle.gimbal", 9)] {
        engine.request_lease(1, scope);
        let grant = wire::Envelope {
            schema_version: 1,
            payload: Some(wire::envelope::Payload::LeaseResponse(
                wire::LeaseResponse {
                    vehicle: Some(wire::VehicleId { value: 1 }),
                    scope: Some(wire::ScopeId {
                        value: scope.into(),
                    }),
                    granted: true,
                    generation: Some(wire::Generation { value: generation }),
                    reason: 0,
                },
            )),
        };
        engine.handle(
            TransportEvent::BootstrapReceived(pilotage_protocol::encode_envelope_length_delimited(
                &grant,
            )),
            0,
        );
    }
    Link {
        engine,
        control: ControlCoordinator::new(),
        feed: None,
        delivery: DeliveryQueue::start(Arc::new(SilentObserver)),
        started: Instant::now(),
        retry_at_ms: None,
        stopped: false,
        stats: LinkStats::default(),
        capture_active: false,
        announced_device: String::new(),
        gated_ticks: 0,
    }
}

/// The scope inside the first datagram action, or a panic that names
/// what arrived instead.
fn datagram_scope(actions: &[ClientAction]) -> String {
    let ClientAction::SendDatagram(bytes) = actions
        .iter()
        .find(|action| matches!(action, ClientAction::SendDatagram(_)))
        .expect("a datagram leaves")
    else {
        panic!("filtered to datagrams");
    };
    let envelope = wire::Envelope::decode(bytes.as_slice()).expect("frame decodes");
    let Some(wire::envelope::Payload::ControlFrame(frame)) = envelope.payload else {
        panic!("the datagram is a control frame");
    };
    frame.scope.expect("frame carries a scope").value
}

#[test]
fn motion_demand_rides_the_motion_lane_even_with_a_gimbal_lease_held() {
    let mut link = admitted_link_with_two_lanes();
    let actions = link.motion_actions(MotionDemand {
        roll: 0.0,
        pitch: 0.5,
        throttle: 0.0,
        yaw: 0.0,
    });
    assert_eq!(datagram_scope(&actions), "vehicle.motion");
}

#[test]
fn arm_rides_the_motion_lane_even_with_a_gimbal_lease_held() {
    let mut link = admitted_link_with_two_lanes();
    let actions = link.action_actions(1);
    let ClientAction::SendBootstrap(bytes) = actions.first().expect("the action is sent") else {
        panic!("actions ride the reliable stream");
    };
    let (envelope, _) = pilotage_protocol::decode_envelope_length_delimited(bytes)
        .expect("action envelope decodes");
    let Some(wire::envelope::Payload::ControlActionCommand(command)) = envelope.payload else {
        panic!("the action rides a control action command");
    };
    assert_eq!(
        command.scope.expect("command carries a scope").value,
        "vehicle.motion"
    );
    assert_eq!(
        command.request.expect("command carries the request").action,
        1
    );
}
