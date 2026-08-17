//! Multi-lane fencing: independent scopes, one activation
//! announcement, revocation isolation, and quiet-denial behavior.

#![allow(clippy::expect_used, clippy::panic)]

use pilotage_protocol::wire;
use prost::Message;

use super::{admit, engine, grant, grant_scope};
use crate::{ClientAction, ControlCommand, ModuleEvent, StreamId, TransportEvent};

/// Decodes the control frame inside a SendDatagram action.
fn frame_of(action: &ClientAction) -> wire::ControlFrame {
    let ClientAction::SendDatagram(bytes) = action else {
        panic!("expected a datagram");
    };
    let envelope = wire::Envelope::decode(bytes.as_slice()).expect("frame decodes");
    let Some(wire::envelope::Payload::ControlFrame(frame)) = envelope.payload else {
        panic!("the datagram is a control frame");
    };
    frame
}

#[test]
fn two_scopes_hold_independent_lanes_under_one_announcement() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    engine.request_lease(1, "vehicle.motion");
    let motion_grant = grant(&mut engine, 4);
    engine.request_lease_quiet(1, "vehicle.gimbal");
    let gimbal_grant = grant_scope(&mut engine, "vehicle.gimbal", 9);

    // The activation announcement travels with the FIRST grant only; the
    // second lane binds to the same announced identity.
    let announcements = |actions: &[ClientAction]| {
        actions
            .iter()
            .filter(|action| {
                let ClientAction::SendBootstrap(bytes) = action else {
                    return false;
                };
                let (envelope, _) = pilotage_protocol::decode_envelope_length_delimited(bytes)
                    .expect("bootstrap decodes");
                matches!(
                    envelope.payload,
                    Some(wire::envelope::Payload::ProfileActivation(_))
                )
            })
            .count()
    };
    assert_eq!(announcements(&motion_grant), 1);
    assert_eq!(announcements(&gimbal_grant), 0);

    // Frames route by scope, each on its own fencing and sequence.
    let motion = frame_of(
        &engine.control_frame(
            1,
            "vehicle.motion",
            ControlCommand::Legacy(wire::ControlPayload::default()),
            10,
        )[0],
    );
    let gimbal_one = frame_of(
        &engine.control_frame(
            1,
            "vehicle.gimbal",
            ControlCommand::Legacy(wire::ControlPayload::default()),
            11,
        )[0],
    );
    let gimbal_two = frame_of(
        &engine.control_frame(
            1,
            "vehicle.gimbal",
            ControlCommand::Legacy(wire::ControlPayload::default()),
            12,
        )[0],
    );
    assert_eq!(
        motion.scope.as_ref().map(|s| s.value.as_str()),
        Some("vehicle.motion")
    );
    assert_eq!(motion.generation.as_ref().map(|g| g.value), Some(4));
    assert_eq!(
        gimbal_one.scope.as_ref().map(|s| s.value.as_str()),
        Some("vehicle.gimbal")
    );
    assert_eq!(gimbal_one.generation.as_ref().map(|g| g.value), Some(9));
    assert_eq!(motion.sequence.as_ref().map(|s| s.value), Some(1));
    assert_eq!(gimbal_one.sequence.as_ref().map(|s| s.value), Some(1));
    assert_eq!(gimbal_two.sequence.as_ref().map(|s| s.value), Some(2));
}

#[test]
fn revoking_one_scope_closes_only_its_lane() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    engine.request_lease(1, "vehicle.motion");
    grant(&mut engine, 4);
    engine.request_lease_quiet(1, "vehicle.gimbal");
    grant_scope(&mut engine, "vehicle.gimbal", 9);

    let revoked = wire::Envelope {
        schema_version: 1,
        payload: Some(wire::envelope::Payload::AuthorityEvent(
            wire::AuthorityEvent {
                event: Some(wire::authority_event::Event::ScopeLeaseRevoked(
                    wire::ScopeLeaseRevoked {
                        vehicle: Some(wire::VehicleId { value: 1 }),
                        scope: Some(wire::ScopeId {
                            value: "vehicle.gimbal".into(),
                        }),
                        generation: Some(wire::Generation { value: 10 }),
                        ..Default::default()
                    },
                )),
            },
        )),
    };
    let bytes = pilotage_protocol::encode_envelope_length_delimited(&revoked);
    engine.handle(TransportEvent::UniStreamOpened(StreamId(9)), 0);
    let mut tagged = vec![0x01];
    tagged.extend_from_slice(&bytes);
    engine.handle(TransportEvent::UniStreamReceived(StreamId(9), tagged), 0);

    assert!(
        !engine.holds(1, "vehicle.gimbal"),
        "the revoked lane closes"
    );
    assert!(engine.holds(1, "vehicle.motion"), "the sibling lane stands");
    assert!(
        !engine
            .control_frame(
                1,
                "vehicle.motion",
                ControlCommand::Legacy(wire::ControlPayload::default()),
                13,
            )
            .is_empty(),
        "motion frames keep flowing"
    );
}

#[test]
fn a_quiet_denial_reports_and_never_asks_the_holder() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    engine.request_lease_quiet(1, "vehicle.gimbal");
    let denial = wire::Envelope {
        schema_version: 1,
        payload: Some(wire::envelope::Payload::LeaseResponse(
            wire::LeaseResponse {
                vehicle: Some(wire::VehicleId { value: 1 }),
                scope: Some(wire::ScopeId {
                    value: "vehicle.gimbal".into(),
                }),
                granted: false,
                generation: Some(wire::Generation { value: 3 }),
                reason: 1,
            },
        )),
    };
    let bytes = pilotage_protocol::encode_envelope_length_delimited(&denial);
    let actions = engine.handle(TransportEvent::BootstrapReceived(bytes), 0);
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, ClientAction::SendBootstrap(_))),
        "a quiet denial sends nothing back"
    );
    assert!(actions.iter().any(|action| matches!(
        action,
        ClientAction::Emit(ModuleEvent::Lease(response)) if !response.granted
    )));
}

#[test]
fn actions_across_lanes_share_one_correlation_id_space() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    engine.request_lease(1, "vehicle.motion");
    grant(&mut engine, 4);
    engine.request_lease_quiet(1, "vehicle.gimbal");
    grant_scope(&mut engine, "vehicle.gimbal", 9);

    let id_of = |actions: &[ClientAction]| {
        let ClientAction::SendBootstrap(bytes) = &actions[0] else {
            panic!("actions ride the reliable stream");
        };
        let (envelope, _) =
            pilotage_protocol::decode_envelope_length_delimited(bytes).expect("action decodes");
        let Some(wire::envelope::Payload::ControlActionCommand(command)) = envelope.payload else {
            panic!("a control action command");
        };
        command.request.expect("request present").action_id
    };
    let arm = engine.control_action(
        1,
        "vehicle.motion",
        wire::ControlActionRequest {
            action: 1,
            mode_target: 0,
            action_id: 0,
        },
    );
    let recenter = engine.control_action(
        1,
        "vehicle.gimbal",
        wire::ControlActionRequest {
            action: 4,
            mode_target: 0,
            action_id: 0,
        },
    );
    // The host correlates per client, not per lane: the same id from a
    // second lane reads as a replay with different content.
    assert_ne!(id_of(&arm), id_of(&recenter));
}
